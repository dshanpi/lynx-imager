//! Fixed firmware recipe using existing SSH trust, credentials, task history and workspace lease.
use super::{
    commands, credentials, firmware_applications as package, firmware_lifecycle as life, ssh,
    storage,
    types::{HostConfig, ProjectConfig, SshCredentials},
    WorkbenchState,
};
use serde_json::{json, Value};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};
use tauri::{AppHandle, State};

static BUILD_START_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn context(
    state: &WorkbenchState,
    device_id: i64,
    project_id: i64,
) -> Result<(HostConfig, ProjectConfig), String> {
    let db = state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?;
    let project =
        storage::project_by_id(&db, project_id).map_err(|_| "FIRMWARE_PROJECT_NOT_FOUND")?;
    if project.device_id != Some(device_id)
        || !project.sdk_dir.starts_with('/')
        || project.sdk_dir.contains('\n')
    {
        return Err("FIRMWARE_PROJECT_SCOPE_MISMATCH".into());
    }
    let host = storage::host_by_id(&db, project.host_id).map_err(|_| "FIRMWARE_HOST_NOT_FOUND")?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED".into());
    }
    Ok((host, project))
}
#[tauri::command]
pub fn workbench_firmware_build_start(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    project_id: i64,
    request_id: String,
    credentials: Option<SshCredentials>,
) -> Result<Value, String> {
    let _start_guard = BUILD_START_LOCK.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?;
    if request_id.len() < 8
        || request_id.len() > 80
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("FIRMWARE_REQUEST_ID_INVALID".into());
    }
    let d = life::board(&state, device_id)?;
    let (host, mut project) = context(&state, device_id, project_id)?;
    let task_id = format!("fwbuild-{request_id}");
    if let Ok(existing) = life::load::<Value>(&state, &task_id, "build", &d.board_uid) {
        if existing["projectId"] == project_id {
            return Ok(existing);
        }
        return Err("FIRMWARE_REQUEST_SCOPE_MISMATCH".into());
    }
    {
        let db = state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?;
        let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM task_history WHERE project_id=?1 AND status='running' AND kind='build')",[project_id],|r|r.get(0)).map_err(|_|"FIRMWARE_STORE_FAILED")?;
        if busy {
            return Err("FIRMWARE_BUILD_ALREADY_RUNNING".into());
        }
    }
    let resolved = credentials::resolve(host.id, credentials.unwrap_or_default())?;
    let command = format!(
        "bash -c {}",
        ssh::shell_quote(&format!(
            "set -e; . ./env.sh; exec python3 -c {} {}",
            ssh::shell_quote(include_str!("firmware_build_recipe.py")),
            ssh::shell_quote(&task_id)
        ))
    );
    project.build_command = command;
    let record = json!({"id":task_id,"deviceId":device_id,"boardUid":d.board_uid,"projectId":project_id,"hostId":project.host_id,"sdkDir":project.sdk_dir,"hostIdentity":{"address":host.address,"port":host.port,"username":host.username,"fingerprint":host.host_fingerprint},"recipe":"nekobox-xip-v1","status":"running","receipt":null,"createdAt":life::now()});
    life::insert(&state, &task_id, "build", &d.board_uid, &record)?;
    let start_failed = |e: String| {
        let mut failed = record.clone();
        failed["status"] = json!("failed");
        failed["error"] = json!(crate::supervisor::redact_learning_context(&e));
        let _ = life::update(&state, &task_id, "build", &d.board_uid, &failed);
        e
    };
    let audit = commands::begin_remote_task_audit(
        &app,
        &task_id,
        "build",
        commands::RemoteTaskActor::User,
        &host,
        Some(&project),
        json!({"recipe":"nekobox-xip-v1"}),
    )
    .map_err(&start_failed)?;
    commands::insert_task(&state, &task_id, "build", Some(project_id)).map_err(&start_failed)?;
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|_| "FIRMWARE_STORE_BUSY")?
        .insert(task_id.clone(), cancellation.clone());
    let shared = state.inner().clone();
    std::thread::spawn(move || {
        let result = commands::run_build_task(
            &app,
            &task_id,
            &host,
            &project,
            &resolved,
            &cancellation,
            &audit,
        );
        let (status, code, error) = match &result {
            Ok(0) => ("success", Some(0), String::new()),
            Ok(code) => ("failed", Some(*code), "FIRMWARE_BUILD_FAILED".into()),
            Err(e) => ("failed", None, e.clone()),
        };
        commands::finish_task(&shared, &task_id, status, code, &error);
        commands::finish_remote_task_audit(&app, &audit, status, &result);
        if let Ok(mut r) = life::load::<Value>(&shared, &task_id, "build", &d.board_uid) {
            r["status"] = json!(status);
            r["error"] = json!(crate::supervisor::redact_learning_context(&error));
            let _ = life::update(&shared, &task_id, "build", &d.board_uid, &r);
        }
        commands::emit_status(
            &app,
            &task_id,
            "build",
            status,
            &crate::supervisor::redact_learning_context(&error),
        );
        if let Ok(mut map) = shared.cancellations.lock() {
            map.remove(&task_id);
        }
    });
    Ok(record)
}
fn remote_receipt(
    state: &WorkbenchState,
    device_id: i64,
    build_id: &str,
    supplied: Option<SshCredentials>,
) -> Result<Value, String> {
    let d = life::board(state, device_id)?;
    let mut record: Value = life::load(state, build_id, "build", &d.board_uid)?;
    let project_id = record["projectId"]
        .as_i64()
        .ok_or("FIRMWARE_BUILD_INVALID")?;
    let (host, project) = context(state, device_id, project_id)?;
    if record["sdkDir"] != project.sdk_dir
        || record["hostId"] != project.host_id
        || record["hostIdentity"]
            != json!({"address":host.address,"port":host.port,"username":host.username,"fingerprint":host.host_fingerprint})
    {
        return Err("FIRMWARE_BUILD_CONTEXT_CHANGED".into());
    }
    let creds = credentials::resolve(host.id, supplied.unwrap_or_default())?;
    let (session, _, _) = ssh::connect(&host, &creds)?;
    let sftp = session.sftp().map_err(|_| "FIRMWARE_SFTP_FAILED")?;
    let path = format!(
        "{}/.lynx-firmware/{}/receipt.json",
        project.sdk_dir.trim_end_matches('/'),
        build_id
    );
    let mut bytes = vec![];
    sftp.open(Path::new(&path))
        .map_err(|_| "FIRMWARE_BUILD_RECEIPT_PENDING")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "FIRMWARE_BUILD_RECEIPT_FAILED")?;
    if bytes.len() > 1024 * 1024 {
        return Err("FIRMWARE_BUILD_RECEIPT_TOO_LARGE".into());
    }
    let receipt: Value =
        serde_json::from_slice(&bytes).map_err(|_| "FIRMWARE_BUILD_RECEIPT_INVALID")?;
    validate_receipt(&receipt, build_id, false)?;
    record["receipt"] = receipt.clone();
    record["status"] = receipt["status"].clone();
    // A stale remote receipt does not prove the process survived a lost SSH session.
    // Preserve the original task failure; explicit refresh may later recover a terminal receipt.
    let task = storage::task_by_id(
        &*state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?,
        build_id,
    )
    .map_err(|_| "FIRMWARE_STORE_FAILED")?;
    if receipt["status"] == "running" && task.as_ref().is_some_and(|t| t.status != "running") {
        record["status"] = json!("unknown");
        record["error"] = json!("FIRMWARE_BUILD_PROCESS_STATE_UNKNOWN");
    }

    record["checkedAt"] = json!(life::now());
    if let Ok(mut log) = sftp.open(Path::new(&format!(
        "{}/.lynx-firmware/{}/build.log",
        project.sdk_dir.trim_end_matches('/'),
        build_id
    ))) {
        let length = log.stat().ok().and_then(|s| s.size).unwrap_or(0);
        let _ = log.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)));
        let mut tail = Vec::new();
        if log.take(64 * 1024).read_to_end(&mut tail).is_ok() {
            record["logTail"] = json!(crate::supervisor::redact_learning_context(
                &String::from_utf8_lossy(&tail)
            ));
        }
    }
    life::update(state, build_id, "build", &d.board_uid, &record)?;
    Ok(record)
}
pub fn validate_receipt(r: &Value, id: &str, complete: bool) -> Result<(), String> {
    if r["schemaVersion"] != 1
        || r["id"] != id
        || r["recipe"] != "nekobox-xip-v1"
        || !matches!(r["status"].as_str(), Some("running" | "failed" | "success"))
    {
        return Err("FIRMWARE_BUILD_RECEIPT_INVALID".into());
    }
    if complete {
        if r["status"] != "success"
            || r["stages"]
                .as_array()
                .is_none_or(|a| a.len() < 6 || a.iter().any(|v| v["status"] != "success"))
        {
            return Err("FIRMWARE_BUILD_NOT_COMPLETE".into());
        }
        for name in [
            "source_lock",
            "application",
            "boot_config",
            "boot",
            "package",
            "structure",
        ] {
            if r["stages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["name"] == name && s["status"] == "success")
                .count()
                != 1
            {
                return Err("FIRMWARE_BUILD_NOT_COMPLETE".into());
            }
        }
        for source in ["zephyr", "zephyr-components", "SyterKit"] {
            let c = r["inputs"][source]["commit"].as_str().unwrap_or_default();
            if c.len() != 40 || !c.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("FIRMWARE_BUILD_INPUTS_MISSING".into());
            }
        }
        if r["toolchain"]["sha256"].as_str().unwrap_or_default().len() != 64
            || r["inputs"]["westManifestSha256"]
                .as_str()
                .unwrap_or_default()
                .len()
                != 64
        {
            return Err("FIRMWARE_BUILD_INPUTS_MISSING".into());
        }
        let a = r["artifacts"]
            .as_array()
            .ok_or("FIRMWARE_BUILD_ARTIFACTS_MISSING")?;
        for file in [
            "nekobox-full.img",
            "nekobox.config",
            "nekobox.dts",
            "xip-boot_spi.bin",
        ] {
            if !a.iter().any(|v| {
                v["fileName"] == file
                    && v["sha256"].as_str().is_some_and(|s| s.len() == 64)
                    && v["sizeBytes"].as_u64().is_some_and(|s| s > 0)
            }) {
                return Err("FIRMWARE_BUILD_ARTIFACTS_MISSING".into());
            }
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn workbench_firmware_build_status(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    build_id: String,
    credentials: Option<SshCredentials>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        remote_receipt(&shared, device_id, &build_id, credentials)
    })
    .await
    .map_err(|_| "FIRMWARE_BUILD_STATUS_FAILED")?
}
#[tauri::command]
pub async fn workbench_firmware_build_candidate(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    build_id: String,
    loader_path: String,
    credentials: Option<SshCredentials>,
) -> Result<life::Candidate, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = remote_receipt(&shared, device_id, &build_id, credentials.clone())?;
        let receipt = &record["receipt"];
        validate_receipt(receipt, &build_id, true)?;
        let artifact = receipt["artifacts"]
            .as_array()
            .ok_or("FIRMWARE_BUILD_ARTIFACTS_MISSING")?
            .iter()
            .find(|v| v["fileName"] == "nekobox-full.img")
            .ok_or("FIRMWARE_BUILD_ARTIFACTS_MISSING")?;
        let (host, mut project) = context(
            &shared,
            device_id,
            record["projectId"]
                .as_i64()
                .ok_or("FIRMWARE_BUILD_INVALID")?,
        )?;
        let supplied = Path::new(&loader_path)
            .canonicalize()
            .map_err(|_| "FIRMWARE_LOADER_MISSING")?;
        package::verify_file(
            supplied.parent().ok_or("FIRMWARE_LOADER_MISSING")?,
            &package::CandidateFile {
                file_name: supplied
                    .file_name()
                    .ok_or("FIRMWARE_LOADER_MISSING")?
                    .to_string_lossy()
                    .into_owned(),
                size_bytes: 4961280,
                sha256: life::F101_LOADER_SHA.into(),
            },
        )?;
        let dir = life::cache_root(&app)?.join(life::id("build-download"));
        std::fs::create_dir(&dir).map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?;
        project.firmware_path = format!(
            "{}/.lynx-firmware/{build_id}/nekobox-full.img",
            project.sdk_dir.trim_end_matches('/')
        );
        project.local_download_dir = dir.to_string_lossy().into_owned();
        project.verify_sha256 = true;
        let transfer = life::id("transfer");
        let creds = credentials::resolve(host.id, credentials.unwrap_or_default())?;
        let audit = commands::begin_remote_task_audit(
            &app,
            &transfer,
            "download",
            commands::RemoteTaskActor::User,
            &host,
            Some(&project),
            json!({"firmwareBuildId":build_id}),
        )?;
        commands::insert_task(&shared, &transfer, "transfer", project.id)?;
        let result = commands::download_task(
            &app,
            &transfer,
            &host,
            &project,
            &creds,
            &AtomicBool::new(false),
            &audit,
        )
        .and_then(|path| {
            let image: package::CandidateFile = serde_json::from_value(artifact.clone())
                .map_err(|_| "FIRMWARE_BUILD_ARTIFACT_INVALID")?;
            package::verify_file(
                &dir.canonicalize()
                    .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?,
                &image,
            )?;
            Ok(path)
        });
        commands::finish_task(
            &shared,
            &transfer,
            if result.is_ok() { "success" } else { "failed" },
            None,
            result
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or_default(),
        );
        commands::finish_remote_task_audit(
            &app,
            &audit,
            if result.is_ok() { "success" } else { "failed" },
            &result,
        );
        let _image = result?;
        let image: package::CandidateFile = serde_json::from_value(artifact.clone())
            .map_err(|_| "FIRMWARE_BUILD_ARTIFACT_INVALID")?;
        package::verify_file(
            &dir.canonicalize()
                .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?,
            &image,
        )?;
        let loader = package::CandidateFile {
            file_name: "f101-ram-loader.img".into(),
            size_bytes: 4961280,
            sha256: life::F101_LOADER_SHA.into(),
        };
        std::fs::copy(&loader_path, dir.join(&loader.file_name))
            .map_err(|_| "FIRMWARE_LOADER_MISSING")?;
        let manifest = package::FirmwareCandidateManifest {
            schema_version: 1,
            application_id: "nekobox".into(),
            version: build_id.clone(),
            board_model: "yuzukineko".into(),
            chip: "f101".into(),
            route: "xip_spinor_full".into(),
            flash_capacity_bytes: 16777216,
            layout: package::CandidateLayout {
                boot_offset: 0,
                parameters_offset: 0x20000,
                application_offset: 0x80000,
            },
            image,
            loader,
            loader_source: "Operator supplied F101 RAM FES Loader; redistribution not included"
                .into(),
            source_commit: receipt["inputs"]["zephyr"]["commit"]
                .as_str()
                .ok_or("FIRMWARE_BUILD_INPUTS_MISSING")?
                .into(),
        };
        let path = dir.join("candidate.json");
        std::fs::write(&path, life::encode(&manifest)?)
            .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
        life::import_candidate(&app, &shared, device_id, &path, None, Some(build_id))
    })
    .await
    .map_err(|_| "FIRMWARE_BUILD_IMPORT_FAILED")?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn other_builds_and_partial_receipts_cannot_be_published() {
        let r = json!({"schemaVersion":1,"id":"a","recipe":"nekobox-xip-v1","status":"running"});
        assert!(validate_receipt(&r, "a", false).is_ok());
        assert!(validate_receipt(&r, "b", false).is_err());
        assert!(validate_receipt(&r, "a", true).is_err());
        let mut r = r;
        r["status"] = json!("success");
        assert!(validate_receipt(&r, "a", true).is_err());
    }
}

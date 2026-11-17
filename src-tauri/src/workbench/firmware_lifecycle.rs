//! Application firmware metadata in the existing Workbench database.
//! Execution remains owned by Workbench/Openix and its process-wide guard.
use super::{
    firmware_applications as package, storage,
    types::{DeviceConfig, TaskHistory},
    WorkbenchState,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager, State};

pub const F101_LOADER_SHA: &str =
    "51bf5c40095fc5f3770cac673e18943972876cad6a84d106d6dc7f042d639186";
pub const ACCEPTANCE_ITEMS: &[&str] = &[
    "boot",
    "display",
    "touch",
    "video",
    "audio",
    "network",
    "power_cycle",
];

pub fn migrate(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS firmware_records(
      id TEXT PRIMARY KEY, kind TEXT NOT NULL, board_uid TEXT NOT NULL,
      payload TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS firmware_records_scope ON firmware_records(board_uid,kind,created_at);")
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn id(prefix: &str) -> String {
    super::commands::new_task_id(prefix)
}
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "FIRMWARE_RECORD_INVALID".into())
}
pub fn board(state: &WorkbenchState, device_id: i64) -> Result<DeviceConfig, String> {
    storage::device_by_id(
        &*state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?,
        device_id,
    )
    .map_err(|_| "FIRMWARE_BOARD_NOT_SAVED".into())
}
pub fn insert<T: Serialize>(
    state: &WorkbenchState,
    id: &str,
    kind: &str,
    uid: &str,
    value: &T,
) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|_| "FIRMWARE_STORE_BUSY")?
        .execute(
            "INSERT INTO firmware_records VALUES(?1,?2,?3,?4,?5,?5)",
            params![id, kind, uid, encode(value)?, now()],
        )
        .map_err(|_| "FIRMWARE_RECORD_EXISTS")?;
    Ok(())
}
pub fn load<T: for<'de> Deserialize<'de>>(
    state: &WorkbenchState,
    id: &str,
    kind: &str,
    uid: &str,
) -> Result<T, String> {
    let raw: String = state
        .db
        .lock()
        .map_err(|_| "FIRMWARE_STORE_BUSY")?
        .query_row(
            "SELECT payload FROM firmware_records WHERE id=?1 AND kind=?2 AND board_uid=?3",
            params![id, kind, uid],
            |r| r.get(0),
        )
        .map_err(|_| "FIRMWARE_RECORD_SCOPE_MISMATCH")?;
    serde_json::from_str(&raw).map_err(|_| "FIRMWARE_RECORD_INVALID".into())
}
pub fn update<T: Serialize>(
    state: &WorkbenchState,
    id: &str,
    kind: &str,
    uid: &str,
    value: &T,
) -> Result<(), String> {
    let n=state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?.execute("UPDATE firmware_records SET payload=?1,updated_at=?2 WHERE id=?3 AND kind=?4 AND board_uid=?5",params![encode(value)?,now(),id,kind,uid]).map_err(|_|"FIRMWARE_STORE_FAILED")?;
    if n != 1 {
        return Err("FIRMWARE_RECORD_SCOPE_MISMATCH".into());
    }
    Ok(())
}
pub fn records(state: &WorkbenchState, uid: &str, kind: &str) -> Result<Vec<Value>, String> {
    let db = state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?;
    let mut s=db.prepare("SELECT payload FROM firmware_records WHERE board_uid=?1 AND kind=?2 ORDER BY created_at DESC,id DESC LIMIT 200").map_err(|_|"FIRMWARE_STORE_FAILED")?;
    let rows = s
        .query_map(params![uid, kind], |r| r.get::<_, String>(0))
        .map_err(|_| "FIRMWARE_STORE_FAILED")?;
    rows.map(|r| {
        serde_json::from_str(&r.map_err(|_| "FIRMWARE_STORE_FAILED")?)
            .map_err(|_| "FIRMWARE_RECORD_INVALID".into())
    })
    .collect()
}
pub fn binding_fingerprint(device: &DeviceConfig) -> String {
    digest(
        json!([
            device.board_uid,
            device.usb_binding,
            device.serial_port,
            device.serial_number,
            device.serial_vid,
            device.serial_pid,
            device.serial_interface_number,
            device.baud_rate,
            device.flasher_kind,
            device.flasher_path,
            device.chip_vendor
        ])
        .to_string()
        .as_bytes(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HardwareProfile {
    pub model: String,
    pub revision: String,
    pub chip: String,
    pub capacity_bytes: u64,
    pub display: String,
    pub touch: String,
    pub operator: String,
    pub evidence: String,
    #[serde(default)]
    pub binding_fingerprint: String,
    #[serde(default)]
    pub recorded_at: i64,
}
fn validate_profile(p: &HardwareProfile) -> Result<(), String> {
    if p.model != "yuzukineko"
        || p.chip != "f101"
        || p.capacity_bytes != 16777216
        || p.display != "1024x768"
        || p.touch != "gt967"
    {
        return Err("FIRMWARE_PROFILE_UNSUPPORTED".into());
    }
    for (s, max) in [(&p.revision, 100), (&p.operator, 100), (&p.evidence, 2000)] {
        if s.trim().is_empty() || s.len() > max || s.chars().any(char::is_control) {
            return Err("FIRMWARE_PROFILE_EVIDENCE_REQUIRED".into());
        }
    }
    Ok(())
}
#[tauri::command]
pub fn workbench_firmware_save_profile(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    mut profile: HardwareProfile,
    confirmed: bool,
) -> Result<HardwareProfile, String> {
    if !confirmed {
        return Err("FIRMWARE_CONFIRMATION_REQUIRED".into());
    }
    validate_profile(&profile)?;
    let d = board(&state, device_id)?;
    if d.chip_vendor != "allwinner" || d.flasher_kind != "openixcli" {
        return Err("FIRMWARE_PROFILE_UNSUPPORTED".into());
    }
    profile.operator = crate::supervisor::redact_learning_context(&profile.operator);
    profile.evidence = crate::supervisor::redact_learning_context(&profile.evidence);
    profile.revision = crate::supervisor::redact_learning_context(&profile.revision);
    profile.binding_fingerprint = binding_fingerprint(&d);
    profile.recorded_at = now();
    // Profile revisions are append-only; historical acceptance retains its own profile.
    insert(&state, &id("fwprofile"), "profile", &d.board_uid, &profile)?;
    Ok(profile)
}
pub fn current_profile(
    state: &WorkbenchState,
    d: &DeviceConfig,
) -> Result<HardwareProfile, String> {
    let value = records(state, &d.board_uid, "profile")?
        .into_iter()
        .next()
        .ok_or("FIRMWARE_PROFILE_REQUIRED")?;
    let p: HardwareProfile =
        serde_json::from_value(value).map_err(|_| "FIRMWARE_PROFILE_INVALID")?;
    validate_profile(&p)?;
    if p.binding_fingerprint != binding_fingerprint(d) {
        return Err("FIRMWARE_BINDING_CHANGED".into());
    }
    Ok(p)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub device_id: i64,
    pub board_uid: String,
    pub manifest: package::FirmwareCandidateManifest,
    pub manifest_path: String,
    pub image_path: String,
    pub loader_path: String,
    pub created_at: i64,
    pub origin: Option<Value>,
    pub build_id: Option<String>,
    pub task_ids: Vec<String>,
}
pub fn cache_root(app: &AppHandle) -> Result<PathBuf, String> {
    let p = app
        .path()
        .app_data_dir()
        .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?
        .join("firmware");
    std::fs::create_dir_all(&p).map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?;
    if std::fs::symlink_metadata(&p)
        .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?
        .file_type()
        .is_symlink()
    {
        return Err("FIRMWARE_CACHE_UNAVAILABLE".into());
    }
    Ok(p)
}
pub fn import_candidate(
    app: &AppHandle,
    state: &WorkbenchState,
    device_id: i64,
    path: &Path,
    origin: Option<Value>,
    build_id: Option<String>,
) -> Result<Candidate, String> {
    let d = board(state, device_id)?;
    let inspected = package::inspect_candidate(path, device_id, d.board_uid.clone())?;
    let candidate_id = id("fwcandidate");
    let dir = cache_root(app)?.join(&candidate_id);
    std::fs::create_dir(&dir).map_err(|_| "FIRMWARE_CACHE_CREATE_FAILED")?;
    for (source, name) in [
        (&inspected.image_path, &inspected.manifest.image.file_name),
        (&inspected.loader_path, &inspected.manifest.loader.file_name),
    ] {
        let target = dir.join(name);
        std::fs::copy(source, &target).map_err(|_| "FIRMWARE_CACHE_COPY_FAILED")?;
    }
    let manifest_path = dir.join("candidate.json");
    std::fs::write(&manifest_path, encode(&inspected.manifest)?)
        .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
    let checked = package::inspect_candidate(&manifest_path, device_id, d.board_uid.clone())?;
    let c = Candidate {
        id: candidate_id,
        device_id,
        board_uid: d.board_uid,
        manifest: checked.manifest,
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        image_path: checked.image_path,
        loader_path: checked.loader_path,
        created_at: now(),
        origin,
        build_id,
        task_ids: vec![],
    };
    insert(state, &c.id, "candidate", &c.board_uid, &c)?;
    Ok(c)
}
#[tauri::command]
pub fn workbench_firmware_import(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    manifest_path: String,
) -> Result<Candidate, String> {
    import_candidate(
        &app,
        &state,
        device_id,
        Path::new(&manifest_path),
        None,
        None,
    )
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub id: String,
    pub candidate_id: String,
    pub device_id: i64,
    pub fingerprint: String,
    pub profile: HardwareProfile,
    pub expires_at: i64,
    pub post_action: String,
    pub used: bool,
}
fn check_candidate(c: &Candidate, p: &HardwareProfile) -> Result<(), String> {
    package::validate_manifest(&c.manifest)?;
    validate_profile(p)?;
    if c.manifest.loader.sha256.to_lowercase() != F101_LOADER_SHA {
        return Err("FIRMWARE_LOADER_NOT_VERIFIED".into());
    }
    let actual = package::inspect_candidate(
        Path::new(&c.manifest_path),
        c.device_id,
        c.board_uid.clone(),
    )?;
    if encode(&actual.manifest)? != encode(&c.manifest)?
        || actual.image_path != c.image_path
        || actual.loader_path != c.loader_path
    {
        return Err("FIRMWARE_CANDIDATE_CHANGED".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn workbench_firmware_prepare(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    candidate_id: String,
    post_action: String,
) -> Result<Confirmation, String> {
    if !["none", "reboot", "poweroff"].contains(&post_action.as_str()) {
        return Err("FIRMWARE_POST_ACTION_INVALID".into());
    }
    let d = board(&state, device_id)?;
    let p = current_profile(&state, &d)?;
    if d.usb_binding.trim().is_empty() {
        return Err("FLASH_TARGET_UNBOUND".into());
    }
    let c: Candidate = load(&state, &candidate_id, "candidate", &d.board_uid)?;
    check_candidate(&c, &p)?;
    super::firmware_distribution::ensure_current_origin(&c).await?;
    let confirmation = Confirmation {
        id: id("fwconfirm"),
        candidate_id,
        device_id,
        fingerprint: binding_fingerprint(&d),
        profile: p,
        expires_at: now() + 300,
        post_action,
        used: false,
    };
    insert(
        &state,
        &confirmation.id,
        "confirmation",
        &d.board_uid,
        &confirmation,
    )?;
    Ok(confirmation)
}
fn claim_confirmation(state: &WorkbenchState, uid: &str, id: &str) -> Result<Confirmation, String> {
    let mut db = state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?;
    let tx = db.transaction().map_err(|_| "FIRMWARE_STORE_FAILED")?;
    let raw:String=tx.query_row("SELECT payload FROM firmware_records WHERE id=?1 AND board_uid=?2 AND kind='confirmation'",params![id,uid],|r|r.get(0)).map_err(|_|"FIRMWARE_CONFIRMATION_INVALID")?;
    let mut c: Confirmation =
        serde_json::from_str(&raw).map_err(|_| "FIRMWARE_CONFIRMATION_INVALID")?;
    if c.used || c.expires_at < now() {
        return Err("FIRMWARE_CONFIRMATION_EXPIRED_OR_USED".into());
    }
    c.used = true;
    tx.execute(
        "UPDATE firmware_records SET payload=?1,updated_at=?2 WHERE id=?3",
        params![encode(&c)?, now(), id],
    )
    .map_err(|_| "FIRMWARE_STORE_FAILED")?;
    tx.commit().map_err(|_| "FIRMWARE_STORE_FAILED")?;
    Ok(c)
}
#[tauri::command]
pub async fn workbench_firmware_start(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    confirmation_id: String,
    confirmed: bool,
) -> Result<String, String> {
    if !confirmed {
        return Err("FIRMWARE_CONFIRMATION_REQUIRED".into());
    }
    let d = board(&state, device_id)?;
    let ticket = claim_confirmation(&state, &d.board_uid, &confirmation_id)?;
    let mut c: Candidate = load(&state, &ticket.candidate_id, "candidate", &d.board_uid)?;
    if ticket.device_id != device_id
        || ticket.fingerprint != binding_fingerprint(&d)
        || encode(&current_profile(&state, &d)?)? != encode(&ticket.profile)?
    {
        return Err("FIRMWARE_BINDING_CHANGED".into());
    }
    check_candidate(&c, &ticket.profile)?;
    super::firmware_distribution::ensure_current_origin(&c).await?;
    if binding_fingerprint(&board(&state, device_id)?) != ticket.fingerprint
        || encode(&current_profile(&state, &d)?)? != encode(&ticket.profile)?
    {
        return Err("FIRMWARE_BINDING_CHANGED".into());
    }
    let args = vec![
        "--raw".into(),
        "--lynx-raw-loader".into(),
        c.loader_path.clone(),
        "--lynx-spinor".into(),
        "--post-action".into(),
        ticket.post_action,
    ];
    let task_id = super::flasher::start_flash_with_args(
        device_id,
        c.image_path.clone(),
        Some(args),
        app,
        &state,
        super::flasher::FlashAuditActor::User,
    )?;
    c.task_ids.push(task_id.clone());
    update(&state, &c.id, "candidate", &c.board_uid, &c)
        .map_err(|_| format!("FIRMWARE_TASK_STARTED_RECORD_FAILED:{task_id}:DO_NOT_RETRY"))?;
    insert(
        &state,
        &task_id,
        "flash_context",
        &c.board_uid,
        &json!({"candidateId":c.id,"imageSha256":c.manifest.image.sha256,"profile":ticket.profile,"taskId":task_id,"at":now()}),
    ).map_err(|_|format!("FIRMWARE_TASK_STARTED_RECORD_FAILED:{task_id}:DO_NOT_RETRY"))?;
    Ok(task_id)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceItem {
    pub item: String,
    pub result: String,
    pub evidence: String,
}
fn validate_acceptance(items: &[AcceptanceItem], operator: &str) -> Result<(), String> {
    if operator.trim().is_empty()
        || operator.len() > 100
        || items.is_empty()
        || items.len() > ACCEPTANCE_ITEMS.len()
    {
        return Err("FIRMWARE_ACCEPTANCE_INVALID".into());
    }
    let mut seen = std::collections::HashSet::new();
    for i in items {
        if !ACCEPTANCE_ITEMS.contains(&i.item.as_str())
            || !seen.insert(&i.item)
            || !["pass", "fail", "not_verified"].contains(&i.result.as_str())
            || i.evidence.len() > 2000
            || i.evidence.chars().any(char::is_control)
            || (i.result != "not_verified" && i.evidence.trim().is_empty())
        {
            return Err("FIRMWARE_ACCEPTANCE_INVALID".into());
        }
    }
    Ok(())
}
pub fn task_for_candidate(
    state: &WorkbenchState,
    c: &Candidate,
    task_id: &str,
) -> Result<TaskHistory, String> {
    if !c.task_ids.iter().any(|id| id == task_id) {
        return Err("FIRMWARE_EVIDENCE_SCOPE_MISMATCH".into());
    }
    let t = storage::task_by_id(
        &*state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?,
        task_id,
    )
    .map_err(|_| "FIRMWARE_TASK_NOT_FOUND")?
    .ok_or("FIRMWARE_TASK_NOT_FOUND")?;
    if t.device_id != Some(c.device_id) || t.kind != "flash" {
        return Err("FIRMWARE_EVIDENCE_SCOPE_MISMATCH".into());
    }
    Ok(t)
}
#[tauri::command]
pub fn workbench_firmware_acceptance(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    candidate_id: String,
    task_id: String,
    operator: String,
    items: Vec<AcceptanceItem>,
) -> Result<Value, String> {
    validate_acceptance(&items, &operator)?;
    let operator = crate::supervisor::redact_learning_context(&operator);
    let items = items
        .into_iter()
        .map(|mut i| {
            i.evidence = crate::supervisor::redact_learning_context(&i.evidence);
            i
        })
        .collect::<Vec<_>>();
    let d = board(&state, device_id)?;
    let c: Candidate = load(&state, &candidate_id, "candidate", &d.board_uid)?;
    let t = task_for_candidate(&state, &c, &task_id)?;
    if t.status == "running" {
        return Err("FIRMWARE_TASK_RUNNING".into());
    }
    let context: Value = load(&state, &task_id, "flash_context", &d.board_uid)?;
    let record = json!({"id":id("fwacceptance"),"candidateId":candidate_id,"taskId":task_id,"imageSha256":c.manifest.image.sha256,"profile":context["profile"],"operator":operator,"items":items,"recordedAt":now(),"source":"manual_operator_report"});
    insert(
        &state,
        record["id"].as_str().unwrap(),
        "acceptance",
        &d.board_uid,
        &record,
    )?;
    Ok(record)
}
#[tauri::command]
pub fn workbench_firmware_workspace(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
) -> Result<Value, String> {
    let d = board(&state, device_id)?;
    let candidates = records(&state, &d.board_uid, "candidate")?;
    let mut tasks = vec![];
    for c in &candidates {
        if let Some(ids) = c["taskIds"].as_array() {
            for t in ids {
                if let Some(t) = t.as_str() {
                    if let Some(task) = storage::task_by_id(
                        &*state.db.lock().map_err(|_| "FIRMWARE_STORE_BUSY")?,
                        t,
                    )
                    .map_err(|_| "FIRMWARE_STORE_FAILED")?
                    {
                        tasks.push(task);
                    }
                }
            }
        }
    }
    Ok(
        json!({"deviceId":device_id,"boardUid":d.board_uid,"profile":current_profile(&state,&d).ok(),"candidates":candidates,"tasks":tasks,"acceptance":records(&state,&d.board_uid,"acceptance")?,"builds":records(&state,&d.board_uid,"build")?,"publications":records(&state,&d.board_uid,"publication")?}),
    )
}

#[tauri::command]
pub fn workbench_firmware_report(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    candidate_id: String,
) -> Result<String, String> {
    let d = board(&state, device_id)?;
    let c: Candidate = load(&state, &candidate_id, "candidate", &d.board_uid)?;
    let tasks = c
        .task_ids
        .iter()
        .map(|id| task_for_candidate(&state, &c, id))
        .collect::<Result<Vec<_>, _>>()?;
    let acceptance = records(&state, &d.board_uid, "acceptance")?
        .into_iter()
        .filter(|a| a["candidateId"] == c.id)
        .collect::<Vec<_>>();
    let report = json!({"candidateId":c.id,"manifest":c.manifest,"tasks":tasks,"acceptance":acceptance,"notice":"Hardware acceptance is operator supplied, separate from physical readback and reboot command results."});
    // Redact each string before serializing so JSON escaping cannot hide credentials.
    fn redact(v: Value) -> Value {
        match v {
            Value::String(s) => json!(crate::supervisor::redact_learning_context(&s)),
            Value::Array(a) => Value::Array(a.into_iter().map(redact).collect()),
            Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, redact(v))).collect()),
            v => v,
        }
    }
    serde_json::to_string_pretty(&redact(report)).map_err(|_| "FIRMWARE_REPORT_FAILED".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> WorkbenchState {
        WorkbenchState::open(Path::new(":memory:")).unwrap()
    }
    fn profile() -> HardwareProfile {
        HardwareProfile {
            model: "yuzukineko".into(),
            revision: "1".into(),
            chip: "f101".into(),
            capacity_bytes: 16777216,
            display: "1024x768".into(),
            touch: "gt967".into(),
            operator: "test operator".into(),
            evidence: "silkscreen and BOM inspected".into(),
            binding_fingerprint: "binding".into(),
            recorded_at: now(),
        }
    }
    #[test]
    fn scope_records_never_cross_boards() {
        let s = state();
        insert(&s, "c", "candidate", "a", &json!({"x":1})).unwrap();
        assert!(load::<Value>(&s, "c", "candidate", "b").is_err());
        assert!(load::<Value>(&s, "c", "profile", "a").is_err());
        assert!(insert(&s, "c", "candidate", "a", &json!({"x":2})).is_err());
    }
    #[test]
    fn unknown_profiles_are_rejected() {
        let mut p = profile();
        assert!(validate_profile(&p).is_ok());
        p.capacity_bytes = 32 * 1024 * 1024;
        assert!(validate_profile(&p).is_err());
        p = profile();
        p.evidence.clear();
        assert!(validate_profile(&p).is_err());
    }
    #[test]
    fn ticket_cannot_be_replayed_or_cross_scoped() {
        let s = state();
        let t = Confirmation {
            id: "t".into(),
            candidate_id: "c".into(),
            device_id: 1,
            fingerprint: "a".into(),
            profile: profile(),
            expires_at: now() + 10,
            post_action: "reboot".into(),
            used: false,
        };
        insert(&s, "t", "confirmation", "a", &t).unwrap();
        assert!(claim_confirmation(&s, "b", "t").is_err());
        assert!(claim_confirmation(&s, "a", "t").is_ok());
        assert!(claim_confirmation(&s, "a", "t").is_err());
    }
    #[test]
    fn expired_ticket_fails_without_execution() {
        let s = state();
        let t = Confirmation {
            id: "t".into(),
            candidate_id: "c".into(),
            device_id: 1,
            fingerprint: "a".into(),
            profile: profile(),
            expires_at: now() - 1,
            post_action: "none".into(),
            used: false,
        };
        insert(&s, "t", "confirmation", "a", &t).unwrap();
        assert!(claim_confirmation(&s, "a", "t").is_err());
    }
    #[test]
    fn partial_acceptance_never_implies_other_checks() {
        let items = vec![AcceptanceItem {
            item: "display".into(),
            result: "pass".into(),
            evidence: "Observed image".into(),
        }];
        assert!(validate_acceptance(&items, "operator").is_ok());
        assert_eq!(
            serde_json::to_value(&items)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut no_evidence = items;
        no_evidence[0].evidence.clear();
        assert!(validate_acceptance(&no_evidence, "operator").is_err());
    }
    #[test]
    fn duplicate_acceptance_is_invalid() {
        let i = AcceptanceItem {
            item: "boot".into(),
            result: "not_verified".into(),
            evidence: "".into(),
        };
        assert!(validate_acceptance(&[i.clone(), i], "op").is_err());
    }
}

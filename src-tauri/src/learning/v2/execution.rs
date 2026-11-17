//! Durable, account-scoped Ubuntu project tasks. No assessment authority.
use super::{environment, manifest};
use crate::{
    auth::state::AuthState,
    learning::local::LocalLearningState,
    workbench::{self, WorkbenchState},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::Arc,
};
use tauri::State;

mod context;
pub mod evidence;
mod provenance;
pub mod sync;
pub(super) use context::tutor_snapshot;

const RUNNER: &str = include_str!("project_runner.py");
const BOOTSTRAP: &str = r#"import sys,json,base64,pathlib,hashlib,os,subprocess
data=json.loads(sys.stdin.buffer.read(1048577))
code=base64.b64decode(data['runner'],validate=True)
root=pathlib.Path.home()/'.local/state/lynx/runners'
root.mkdir(parents=True,exist_ok=True,mode=0o700)
if root.is_symlink() or root.stat().st_uid!=os.getuid() or root.stat().st_mode&0o077: raise RuntimeError('unsafe runner directory')
file=root/(hashlib.sha256(code).hexdigest()+'.py')
try:
 fd=os.open(file,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
 with os.fdopen(fd,'wb') as f: f.write(code)
except FileExistsError:
 if file.is_symlink() or file.read_bytes()!=code: raise RuntimeError('runner mismatch')
p=subprocess.run([sys.executable,str(file)],input=json.dumps(data['request']).encode(),stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=120)
sys.stdout.buffer.write(p.stdout[:131072]);sys.exit(p.returncode)
"#;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRequest {
    #[serde(flatten)]
    pub environment: environment::EnvironmentRequest,
    pub unit_id: String,
    pub activity_id: String,
    pub action_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRun {
    pub run_id: String,
    pub request: ActionRequest,
    pub project_id: i64,
    pub project_name: String,
    pub host_name: String,
    pub state: String,
    pub result: Value,
    pub updated_at: String,
    pub sync_status: String,
}
#[derive(Deserialize, Serialize)]
struct Stored {
    #[serde(default)]
    revision: u64,
    run: ActionRun,
    scope: String,
    binding_digest: String,
    host: workbench::types::HostConfig,
}
pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_project_runs(run_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL,enrollment_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,activity_id TEXT NOT NULL,record TEXT NOT NULL,updated_at TEXT NOT NULL);CREATE INDEX IF NOT EXISTS learn_project_runs_owner ON learn_project_runs(user_id,enrollment_id,manifest_sha256,activity_id);")?;
    evidence::migrate(db)?;
    sync::journal::migrate(db)?;
    sync::review_gate::migrate(db)
}
fn store(local: &LocalLearningState, user: i64, s: &mut Stored) -> Result<(), String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let next = s.revision.checked_add(1).ok_or("LEARN_RUN_INVALID")?;
    let mut record = serde_json::to_value(&*s).map_err(|_| "LEARN_RUN_INVALID")?;
    record["revision"] = json!(next);
    let changed = db.execute("INSERT INTO learn_project_runs VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(run_id) DO UPDATE SET record=excluded.record,updated_at=excluded.updated_at WHERE learn_project_runs.user_id=excluded.user_id AND COALESCE(json_extract(learn_project_runs.record,'$.revision'),0)=?8",params![s.run.run_id,user,s.run.request.environment.enrollment_id,s.run.request.environment.manifest_sha256,s.run.request.activity_id,record.to_string(),s.run.updated_at,s.revision]).map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    if changed != 1 {
        return Err("LEARN_RUN_CONFLICT:任务记录已更新，请查询原任务".into());
    }
    s.revision = next;
    Ok(())
}
fn load(local: &LocalLearningState, user: i64, id: &str) -> Result<Stored, String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let raw: String = db
        .query_row(
            "SELECT record FROM learn_project_runs WHERE user_id=?1 AND run_id=?2",
            params![user, id],
            |r| r.get(0),
        )
        .map_err(|_| "LEARN_RUN_NOT_FOUND")?;
    serde_json::from_str(&raw).map_err(|_| "LEARN_RUN_INVALID".into())
}
async fn user(auth: &AuthState) -> Result<i64, String> {
    Ok(auth.get_user().await?.ok_or("LEARN_AUTH_REQUIRED")?.id)
}
fn task(course: &manifest::VerifiedCourse, r: &ActionRequest) -> Result<(String, String), String> {
    if !course.missing_capabilities.is_empty() {
        return Err("LEARN_ACTION_UNSUPPORTED:当前客户端不具备课程要求的全部能力".into());
    }
    let unit = course
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .find(|u| u.unit_id == r.unit_id)
        .ok_or("LEARN_UNIT_INVALID")?;
    let activity = unit
        .activities
        .iter()
        .find(|a| {
            a.activity_id == r.activity_id
                && a.environment_ref == r.environment.profile_id
                && a.action_refs.contains(&r.action_id)
        })
        .ok_or("LEARN_ACTION_NOT_ALLOWED")?;
    let action = course
        .manifest
        .actions
        .iter()
        .find(|a| a.action_id == r.action_id)
        .ok_or("LEARN_ACTION_INVALID")?;
    if course.manifest.schema_version != 2
        || action.capability_id.as_deref() != Some("learn.project.task")
        || action.catalog_version != "1.0.0"
        || action.target != "ubuntu"
        || action.risk != "workspace_write"
    {
        return Err(
            "LEARN_ACTION_UNSUPPORTED:本版本仅支持已审核的 Ubuntu 工程任务，不能替代板卡操作"
                .into(),
        );
    }
    let profile = course
        .manifest
        .environments
        .iter()
        .find(|e| e.profile_id == r.environment.profile_id)
        .ok_or("LEARN_ENVIRONMENT_INVALID")?;
    if profile.target != "ubuntu" {
        return Err("LEARN_ACTION_TARGET_MISMATCH".into());
    }
    if action.parameters.len() != 2 {
        return Err("LEARN_ACTION_PARAMETERS_INVALID".into());
    }
    let resource = action
        .parameters
        .get("projectRef")
        .and_then(Value::as_str)
        .ok_or("LEARN_PROJECT_REFERENCE_INVALID")?;
    let task_id = action
        .parameters
        .get("taskId")
        .and_then(Value::as_str)
        .filter(|v| manifest::valid_id(v))
        .ok_or("LEARN_TASK_ID_INVALID")?;
    if !activity.resource_refs.iter().any(|r| r == resource) {
        return Err("LEARN_PROJECT_REFERENCE_INVALID".into());
    }
    let source = course
        .manifest
        .resources
        .iter()
        .find(|r| r.resource_id == resource && r.kind == "source")
        .ok_or("LEARN_PROJECT_REFERENCE_INVALID")?;
    let commit = source
        .commit
        .as_deref()
        .filter(|c| [40, 64].contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("LEARN_SOURCE_COMMIT_REQUIRED")?;
    Ok((commit.into(), task_id.into()))
}
async fn remote(host: workbench::types::HostConfig, request: Value) -> Result<Value, String> {
    let credentials = workbench::credentials::resolve(host.id, Default::default())?;
    tauri::async_runtime::spawn_blocking(move || {
        let (ssh, _, _) = workbench::ssh::connect(&host, &credentials)?;
        ssh.set_timeout(130_000);
        let mut channel = ssh.channel_session().map_err(|_| "LEARN_RUN_CHANNEL")?;
        channel
            .exec(&format!(
                "python3 -c {}",
                workbench::ssh::shell_quote(BOOTSTRAP)
            ))
            .map_err(|_| "LEARN_RUN_EXEC")?;
        let bytes =
            serde_json::to_vec(&json!({"runner":STANDARD.encode(RUNNER),"request":request}))
                .map_err(|_| "LEARN_RUN_REQUEST")?;
        channel.write_all(&bytes).map_err(|_| "LEARN_RUN_SEND")?;
        channel.send_eof().map_err(|_| "LEARN_RUN_SEND")?;
        let mut output = Vec::new();
        Read::by_ref(&mut channel)
            .take(131073)
            .read_to_end(&mut output)
            .map_err(|_| "LEARN_RUN_CONNECTION_LOST:连接中断，请查询原任务，不要重新执行")?;
        if output.len() > 131072 {
            return Err("LEARN_RUN_OUTPUT_LIMIT".into());
        }
        channel.wait_close().map_err(|_| "LEARN_RUN_CLOSE")?;
        let result: Value =
            serde_json::from_slice(&output).map_err(|_| "LEARN_RUN_RESPONSE_INVALID")?;
        if let Some(code) = result.get("error").and_then(Value::as_str) {
            if code.len() <= 128 && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') {
                return Err(format!("LEARN_RUN_{code}"));
            }
            return Err("LEARN_RUN_FAILED".into());
        }
        if channel.exit_status().map_err(|_| "LEARN_RUN_STATUS")? != 0 {
            return Err("LEARN_RUN_FAILED".into());
        }
        Ok(result)
    })
    .await
    .map_err(|_| "LEARN_RUN_WORKER_FAILED")?
}
fn receive(s: &mut Stored, mut result: Value) -> Result<(), String> {
    let state = result
        .get("state")
        .and_then(Value::as_str)
        .ok_or("LEARN_RUN_RESPONSE_INVALID")?;
    if result.get("runId").and_then(Value::as_str) != Some(&s.run.run_id)
        || ![
            "prepared",
            "starting",
            "running",
            "cancelling",
            "succeeded",
            "failed",
            "cancelled",
            "timed_out",
            "output_limit",
            "busy",
            "unknown",
        ]
        .contains(&state)
    {
        return Err("LEARN_RUN_RESPONSE_INVALID".into());
    }
    s.run.state = state.into();
    if let Some(log) = result.get("log").and_then(Value::as_str) {
        result["log"] = json!(super::tutor::redact_task_log(log));
    }
    s.run.result = result;
    s.run.updated_at = chrono::Utc::now().to_rfc3339();
    Ok(())
}

fn receive_preparation(s: &mut Stored, response: Result<Value, String>) -> Result<(), String> {
    let result = response.and_then(|value| receive(s, value));
    if result.is_err() {
        // No process can start during preparation, including malformed responses.
        s.run.state = "failed".into();
        s.run.result = json!({"reason":"PREPARATION_FAILED"});
        s.run.updated_at = chrono::Utc::now().to_rfc3339();
    }
    result
}

#[tauri::command]
pub async fn learn_prepare_action(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: ActionRequest,
) -> Result<ActionRun, String> {
    let (user_id, course) = environment::verified(&auth, &request.environment).await?;
    let (commit, task_id) = task(&course, &request)?;
    let (binding, project, host) =
        environment::bound(&local, &workbench, user_id, &request.environment)?;
    let version_checks = course
        .manifest
        .environments
        .iter()
        .find(|e| e.profile_id == request.environment.profile_id)
        .ok_or("LEARN_ENVIRONMENT_INVALID")?
        .version_checks
        .clone();
    let run_id = format!("learn_{:032x}", rand::random::<u128>());
    let scope = hex::encode(Sha256::digest(
        serde_json::to_vec(
            &json!({"user":user_id,"request":request,"binding":binding.config_digest}),
        )
        .map_err(|_| "LEARN_RUN_INVALID")?,
    ));
    let mut saved = Stored {
        revision: 0,
        scope: scope.clone(),
        binding_digest: binding.config_digest,
        host: host.clone(),
        run: ActionRun {
            run_id: run_id.clone(),
            request,
            project_id: binding.project_id,
            project_name: binding.project_name,
            host_name: binding.host_name,
            state: "unknown".into(),
            result: json!({}),
            updated_at: chrono::Utc::now().to_rfc3339(),
            sync_status: "local_only".into(),
        },
    };
    store(&local, user_id, &mut saved)?;
    let response = remote(host,json!({"operation":"prepare","runId":run_id,"scope":scope,"workspace":project.sdk_dir,"commit":commit,"taskId":task_id,"versionChecks":version_checks})).await;
    let outcome = receive_preparation(&mut saved, response);
    store(&local, user_id, &mut saved)?;
    outcome?;
    if user(&auth).await? != user_id {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(saved.run)
}

#[tauri::command]
pub async fn learn_start_action(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    run_id: String,
    confirmation: String,
) -> Result<ActionRun, String> {
    let user_id = user(&auth).await?;
    let mut saved = load(&local, user_id, &run_id)?;
    let (checked, course) = environment::verified(&auth, &saved.run.request.environment).await?;
    if checked != user_id {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    task(&course, &saved.run.request)?;
    let (binding, _, _) =
        environment::bound(&local, &workbench, user_id, &saved.run.request.environment)?;
    if binding.config_digest != saved.binding_digest {
        return Err("LEARN_BINDING_CHANGED".into());
    }
    if saved.run.state != "prepared" {
        return Err("LEARN_RUN_ALREADY_DISPATCHED:请查询已有任务，不能重复启动".into());
    }
    if saved.run.result.get("confirmation").and_then(Value::as_str) != Some(&confirmation) {
        return Err("LEARN_CONFIRMATION_MISMATCH".into());
    }
    // Persist dispatch intent before sending. Lost SSH acknowledgement never becomes retryable execution.
    saved.run.state = "unknown".into();
    store(&local, user_id, &mut saved)?;
    let result = remote(
        saved.host.clone(),
        json!({"operation":"start","runId":run_id,"scope":saved.scope,"confirmation":confirmation}),
    )
    .await?;
    receive(&mut saved, result)?;
    store(&local, user_id, &mut saved)?;
    if user(&auth).await? != user_id {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(saved.run)
}

async fn inspect_or_cancel(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    run_id: String,
    cancel: bool,
) -> Result<ActionRun, String> {
    let user_id = user(&auth).await?;
    let mut saved = load(&local, user_id, &run_id)?;
    // Ownership is required; cancellation must still work after an entitlement expires.
    let result = remote(
        saved.host.clone(),
        json!({"operation":if cancel {"cancel"}else{"query"},"runId":run_id,"scope":saved.scope}),
    )
    .await?;
    receive(&mut saved, result)?;
    store(&local, user_id, &mut saved)?;
    if user(&auth).await? != user_id {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(saved.run)
}

#[tauri::command]
pub async fn learn_query_action(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    run_id: String,
) -> Result<ActionRun, String> {
    inspect_or_cancel(auth, local, run_id, false).await
}

#[tauri::command]
pub async fn learn_cancel_action(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    run_id: String,
) -> Result<ActionRun, String> {
    inspect_or_cancel(auth, local, run_id, true).await
}

#[tauri::command]
pub async fn learn_list_actions(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    request: ActionRequest,
) -> Result<Vec<ActionRun>, String> {
    let user_id = user(&auth).await?;
    list_scoped_actions(&local, user_id, &request)
}

fn list_scoped_actions(
    local: &LocalLearningState,
    user_id: i64,
    request: &ActionRequest,
) -> Result<Vec<ActionRun>, String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut stmt=db.prepare("SELECT record FROM learn_project_runs WHERE user_id=?1 AND enrollment_id=?2 AND manifest_sha256=?3 AND activity_id=?4 ORDER BY updated_at DESC LIMIT 50").map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    let rows = stmt
        .query_map(
            params![
                user_id,
                request.environment.enrollment_id,
                request.environment.manifest_sha256,
                request.activity_id
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut runs = Vec::new();
    for row in rows {
        let s: Stored = serde_json::from_str(&row.map_err(|_| "LEARN_STORE_UNAVAILABLE")?)
            .map_err(|_| "LEARN_RUN_INVALID")?;
        let r = &s.run.request;
        if r.environment.enrollment_id == request.environment.enrollment_id
            && r.environment.release_id == request.environment.release_id
            && r.environment.manifest_sha256 == request.environment.manifest_sha256
            && r.environment.profile_id == request.environment.profile_id
            && r.unit_id == request.unit_id
            && r.activity_id == request.activity_id
            && r.action_id == request.action_id
        {
            runs.push(s.run)
        }
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_history_requires_every_course_scope_field() {
        let local =
            LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir())
                .unwrap();
        let mut saved = saved_fixture();
        store(&local, 7, &mut saved).unwrap();
        assert_eq!(
            list_scoped_actions(&local, 7, &saved.run.request)
                .unwrap()
                .len(),
            1
        );
        assert!(list_scoped_actions(&local, 8, &saved.run.request)
            .unwrap()
            .is_empty());
        for field in [
            "enrollmentId",
            "releaseId",
            "manifestSha256",
            "profileId",
            "unitId",
            "activityId",
            "actionId",
        ] {
            let mut value = serde_json::to_value(&saved.run.request).unwrap();
            value[field] = json!("different");
            let request = serde_json::from_value(value).unwrap();
            assert!(
                list_scoped_actions(&local, 7, &request).unwrap().is_empty(),
                "mixed {field}"
            );
        }
    }

    #[test]
    fn task_log_preserves_diagnostics_beyond_tutor_budget_without_secrets() {
        let mut saved = saved_fixture();
        let log = format!(
            "{}\npassword=canary-secret\nfinal compiler error",
            "x".repeat(10000)
        );
        receive(&mut saved, json!({"runId":"test_run","state":"failed","log":log,"logSha256":"f".repeat(64),"logBytes":10100})).unwrap();
        let displayed = saved.run.result["log"].as_str().unwrap();
        assert!(displayed.ends_with("final compiler error"));
        assert!(!displayed.contains("canary-secret"));
        assert!(displayed.len() <= 32768);
        assert_eq!(saved.run.sync_status, "local_only");
        assert_eq!(saved.run.result["logSha256"], "f".repeat(64));
    }

    pub(super) fn saved_fixture() -> Stored {
        serde_json::from_value(json!({
            "run": {"runId":"test_run","request":fixture().1,"projectId":1,
                "projectName":"test","hostName":"test","state":"prepared",
                "result":{"confirmation":"confirmed"},"updatedAt":"before","syncStatus":"local_only"},
            "scope":"a".repeat(64),"binding_digest":"b".repeat(64),
            "host":{"id":1,"name":"test","address":"127.0.0.1","port":22,
                "username":"test","authType":"password","privateKeyPath":"",
                "hostFingerprint":"","defaultWorkDir":"/tmp"}
        })).unwrap()
    }

    #[test]
    fn stale_queries_and_duplicate_dispatch_cannot_overwrite_newer_record() {
        let local =
            LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir())
                .unwrap();
        let mut initial = saved_fixture();
        store(&local, 7, &mut initial).unwrap();
        let mut query = load(&local, 7, "test_run").unwrap();
        let mut start = load(&local, 7, "test_run").unwrap();
        let mut duplicate = load(&local, 7, "test_run").unwrap();
        start.run.state = "unknown".into();
        store(&local, 7, &mut start).unwrap();
        duplicate.run.state = "unknown".into();
        assert!(
            store(&local, 7, &mut duplicate).is_err(),
            "duplicate dispatch reserved the same run"
        );
        assert!(
            store(&local, 7, &mut query).is_err(),
            "stale query erased dispatch intent"
        );
        assert_eq!(load(&local, 7, "test_run").unwrap().run.state, "unknown");
        assert!(load(&local, 8, "test_run").is_err());
        receive(&mut start, json!({"runId":"test_run","state":"succeeded"})).unwrap();
        store(&local, 7, &mut start).unwrap();
        assert_eq!(load(&local, 7, "test_run").unwrap().run.state, "succeeded");
    }

    #[test]
    fn remote_logs_are_redacted_before_persistence_or_display() {
        let mut saved = saved_fixture();
        receive(&mut saved, json!({"runId":"test_run","state":"failed",
            "log":"compiler failed\nAuthorization: Bearer secret-canary-token\npassword=secret-canary-password",
            "logSha256":"f".repeat(64)})).unwrap();
        let log = saved.run.result["log"].as_str().unwrap();
        assert!(log.contains("compiler failed"));
        assert!(!log.contains("secret-canary"));
        assert_eq!(saved.run.result["logSha256"], "f".repeat(64));
    }

    #[test]
    fn legacy_records_without_revision_can_be_updated_without_data_loss() {
        let local =
            LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir())
                .unwrap();
        let mut value = serde_json::to_value(saved_fixture()).unwrap();
        value.as_object_mut().unwrap().remove("revision");
        local.db.lock().unwrap().execute("INSERT INTO learn_project_runs VALUES('test_run',7,'enr','digest','activity',?1,'before')",params![value.to_string()]).unwrap();
        let mut saved = load(&local, 7, "test_run").unwrap();
        assert_eq!(saved.revision, 0);
        saved.run.state = "cancelled".into();
        store(&local, 7, &mut saved).unwrap();
        let loaded = load(&local, 7, "test_run").unwrap();
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.run.result["confirmation"], "confirmed");
        assert_eq!(loaded.run.state, "cancelled");
    }

    #[test]
    fn invalid_preparation_never_reserves_an_unknown_execution() {
        for response in [
            Err("LEARN_RUN_CONNECTION_LOST".into()),
            Ok(json!({"runId":"wrong","state":"prepared"})),
            Ok(json!({"runId":"test_run","state":"unexpected"})),
        ] {
            let mut saved = saved_fixture();
            saved.run.state = "unknown".into();
            assert!(receive_preparation(&mut saved, response).is_err());
            assert_eq!(saved.run.state, "failed");
            assert_eq!(saved.run.result, json!({"reason":"PREPARATION_FAILED"}));
        }
    }

    pub(super) fn fixture() -> (manifest::VerifiedCourse, ActionRequest) {
        let mut c = manifest::tests::verified();
        c.missing_capabilities.clear();
        c.manifest.schema_version = 2;
        let activity = c.manifest.chapters[0].units[0]
            .activities
            .iter()
            .find(|a| !a.action_refs.is_empty())
            .unwrap()
            .clone();
        let action = &mut c.manifest.actions[0];
        action.capability_id = Some("learn.project.task".into());
        action.parameters =
            serde_json::from_value(json!({"projectRef":"source_hello","taskId":"build"})).unwrap();
        let r = ActionRequest {
            environment: environment::EnvironmentRequest {
                enrollment_id: c.enrollment_id.clone(),
                release_id: c.manifest.release_id.clone(),
                manifest_sha256: c.manifest_sha256.clone(),
                profile_id: activity.environment_ref,
            },
            unit_id: c.manifest.chapters[0].units[0].unit_id.clone(),
            activity_id: activity.activity_id,
            action_id: action.action_id.clone(),
        };
        (c, r)
    }

    #[test]
    fn only_course_referenced_pinned_project_task_can_be_prepared() {
        let (mut c, r) = fixture();
        assert_eq!(task(&c, &r).unwrap(), ("c".repeat(40), "build".into()));
        c.manifest.actions[0]
            .parameters
            .insert("shell".into(), json!("echo forbidden"));
        assert!(task(&c, &r).is_err());
        c.manifest.actions[0].parameters.remove("shell");
        c.manifest.resources[1].commit = None;
        assert!(task(&c, &r).is_err());
    }
    #[test]
    fn other_activity_board_or_unknown_capability_cannot_execute() {
        let (mut c, mut r) = fixture();
        let activity = r.activity_id.clone();
        r.activity_id = "not_this_activity".into();
        assert!(task(&c, &r).is_err());
        r.activity_id = activity;
        c.manifest.actions[0].target = "board".into();
        assert!(task(&c, &r).is_err());
        c.manifest.actions[0].target = "ubuntu".into();
        c.missing_capabilities.push("unknown.adapter".into());
        assert!(task(&c, &r).is_err());
    }
    #[test]
    fn local_store_is_additive_and_retains_owner_and_release_boundaries() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE legacy_notes(value TEXT);INSERT INTO legacy_notes VALUES('keep');",
        )
        .unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT value FROM legacy_notes", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "keep"
        );
        db.execute(
            "INSERT INTO learn_project_runs VALUES('run',7,'enr','digest','activity','{}','now')",
            [],
        )
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM learn_project_runs WHERE user_id=8",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}

//! Course-scoped Linux board observations. No deployment or assessment authority.
use super::{environment, execution::ActionRequest, manifest};
use crate::{
    auth::state::AuthState,
    learning::local::LocalLearningState,
    workbench::{self, WorkbenchState},
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

pub(super) fn supported(action: &manifest::CourseAction) -> bool {
    action.capability_id.as_deref() == Some("learn.board.observe")
        && action.catalog_version == "1.0.0"
        && action.target == "board"
        && action.risk == "read_only"
        && action.parameters.is_empty()
        && action.evidence_outputs == ["board_observation"]
}

fn authorize(course: &manifest::VerifiedCourse, request: &ActionRequest) -> Result<(), String> {
    if course.manifest.schema_version != 2
        || !course.missing_capabilities.is_empty()
        || course.enrollment_id != request.environment.enrollment_id
        || course.manifest.release_id != request.environment.release_id
        || course.manifest_sha256 != request.environment.manifest_sha256
    {
        return Err("LEARN_BOARD_ACTION_UNSUPPORTED".into());
    }
    let unit = course
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .find(|u| u.unit_id == request.unit_id)
        .ok_or("LEARN_UNIT_INVALID")?;
    unit.activities
        .iter()
        .find(|a| {
            a.activity_id == request.activity_id
                && a.environment_ref == request.environment.profile_id
                && a.action_refs.contains(&request.action_id)
        })
        .ok_or("LEARN_ACTION_NOT_ALLOWED")?;
    let action = course
        .manifest
        .actions
        .iter()
        .find(|a| a.action_id == request.action_id)
        .ok_or("LEARN_ACTION_INVALID")?;
    if !supported(action)
        || !course
            .manifest
            .environments
            .iter()
            .any(|p| p.profile_id == request.environment.profile_id && p.target == "board")
    {
        return Err("LEARN_BOARD_ACTION_UNSUPPORTED".into());
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ObservationState {
    Unknown,
    Observed,
    Failed,
    Stale,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSync {
    LocalOnly,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationTrust {
    ClientObservation,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardSnapshot {
    transport: String,
    observed_at_ms: u64,
    os_name: String,
    kernel: String,
    architecture: String,
    cpu_model: String,
    cpu_cores: Option<u32>,
    memory_total_bytes: Option<u64>,
    uptime_seconds: Option<u64>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardObservation {
    observation_id: String,
    request: ActionRequest,
    project_id: i64,
    project_name: String,
    board_uid: String,
    board_name: String,
    device_id: i64,
    binding_digest: String,
    state: ObservationState,
    started_at: String,
    finished_at: Option<String>,
    snapshot: Option<BoardSnapshot>,
    evidence_sha256: Option<String>,
    error_code: Option<String>,
    sync_status: LocalSync,
    trust: ObservationTrust,
}

pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_board_observations(observation_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL,enrollment_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,activity_id TEXT NOT NULL,record TEXT NOT NULL,started_at TEXT NOT NULL);CREATE INDEX IF NOT EXISTS learn_board_observation_owner ON learn_board_observations(user_id,enrollment_id,manifest_sha256,activity_id);")
}
fn persist(
    local: &LocalLearningState,
    user: i64,
    record: &BoardObservation,
    insert: bool,
) -> Result<(), String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let encoded = serde_json::to_string(record).map_err(|_| "LEARN_OBSERVATION_INVALID")?;
    let changed = if insert {
        db.execute("INSERT INTO learn_board_observations VALUES(?1,?2,?3,?4,?5,?6,?7)", params![record.observation_id,user,record.request.environment.enrollment_id,record.request.environment.manifest_sha256,record.request.activity_id,encoded,record.started_at])
    } else {
        db.execute("UPDATE learn_board_observations SET record=?1 WHERE observation_id=?2 AND user_id=?3 AND json_extract(record,'$.state')='unknown'", params![encoded,record.observation_id,user])
    }.map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    if changed != 1 {
        return Err("LEARN_OBSERVATION_CONFLICT".into());
    }
    Ok(())
}
fn same_scope(a: &ActionRequest, b: &ActionRequest) -> bool {
    a.environment.enrollment_id == b.environment.enrollment_id
        && a.environment.release_id == b.environment.release_id
        && a.environment.manifest_sha256 == b.environment.manifest_sha256
        && a.environment.profile_id == b.environment.profile_id
        && a.unit_id == b.unit_id
        && a.activity_id == b.activity_id
        && a.action_id == b.action_id
}
fn evidence_digest(record: &BoardObservation) -> Result<String, String> {
    let mut value = serde_json::to_value(record).map_err(|_| "LEARN_OBSERVATION_INVALID")?;
    value
        .as_object_mut()
        .ok_or("LEARN_OBSERVATION_INVALID")?
        .remove("evidenceSha256");
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&value).map_err(|_| "LEARN_OBSERVATION_INVALID")?,
    )))
}
fn validate_record(record: &BoardObservation) -> Result<(), String> {
    let valid = match record.state {
        ObservationState::Unknown => {
            record.snapshot.is_none()
                && record.evidence_sha256.is_none()
                && record.error_code.is_none()
                && record.finished_at.is_none()
        }
        ObservationState::Failed => {
            record.snapshot.is_none()
                && record.evidence_sha256.is_none()
                && record.finished_at.is_some()
                && record.error_code.as_ref().is_some_and(|c| {
                    !c.is_empty()
                        && c.len() <= 96
                        && c.bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                })
        }
        ObservationState::Observed | ObservationState::Stale => {
            record.snapshot.is_some()
                && record.finished_at.is_some()
                && record.error_code.is_none()
                && record.evidence_sha256.as_deref() == Some(evidence_digest(record)?.as_str())
        }
    };
    if !valid {
        return Err("LEARN_OBSERVATION_INVALID".into());
    }
    Ok(())
}
fn history(
    local: &LocalLearningState,
    user: i64,
    request: &ActionRequest,
) -> Result<Vec<BoardObservation>, String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut statement = db.prepare("SELECT record FROM learn_board_observations WHERE user_id=?1 AND enrollment_id=?2 AND manifest_sha256=?3 AND activity_id=?4 AND json_extract(record,'$.request.releaseId')=?5 AND json_extract(record,'$.request.profileId')=?6 AND json_extract(record,'$.request.unitId')=?7 AND json_extract(record,'$.request.actionId')=?8 ORDER BY started_at DESC LIMIT 50").map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let rows = statement
        .query_map(
            params![
                user,
                request.environment.enrollment_id,
                request.environment.manifest_sha256,
                request.activity_id,
                request.environment.release_id,
                request.environment.profile_id,
                request.unit_id,
                request.action_id
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut records = Vec::new();
    for row in rows {
        let record: BoardObservation =
            serde_json::from_str(&row.map_err(|_| "LEARN_STORE_UNAVAILABLE")?)
                .map_err(|_| "LEARN_OBSERVATION_INVALID")?;
        if same_scope(&record.request, request) {
            validate_record(&record)?;
            records.push(record);
        }
    }
    Ok(records)
}
fn snapshot(value: workbench::linux_overview::LinuxDeviceOverview) -> BoardSnapshot {
    let text = |v: &str| super::tutor::redact_context(v).chars().take(512).collect();
    BoardSnapshot {
        transport: value.transport,
        observed_at_ms: value.collected_at_ms,
        os_name: text(&value.os_name),
        kernel: text(&value.kernel),
        architecture: text(&value.architecture),
        cpu_model: text(&value.cpu_model),
        cpu_cores: value.cpu_cores,
        memory_total_bytes: value.memory_total_bytes,
        uptime_seconds: value.uptime_seconds,
    }
}
fn finish(
    record: &mut BoardObservation,
    result: Result<workbench::linux_overview::LinuxDeviceOverview, String>,
    current: bool,
) -> Result<(), String> {
    record.finished_at = Some(chrono::Utc::now().to_rfc3339());
    let result = result.and_then(|v| {
        if v.device_id != record.device_id
            || v.collected_at_ms == 0
            || !["adb", "serial"].contains(&v.transport.as_str())
            || (v.kernel.is_empty() && v.architecture.is_empty() && v.cpu_model.is_empty())
        {
            return Err("LEARN_BOARD_OBSERVATION_INVALID".into());
        }
        Ok(v)
    });
    match result {
        Ok(value) => {
            record.snapshot = Some(snapshot(value));
            record.state = if current {
                ObservationState::Observed
            } else {
                ObservationState::Stale
            };
            record.evidence_sha256 = Some(evidence_digest(record)?);
        }
        Err(error) => {
            record.state = ObservationState::Failed;
            let code = error.split(':').next().unwrap_or_default();
            record.error_code = Some(
                if !code.is_empty()
                    && code.len() <= 96
                    && code
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                {
                    code.into()
                } else {
                    "LEARN_BOARD_OBSERVATION_FAILED".into()
                },
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn learn_list_board_observations(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    request: ActionRequest,
) -> Result<Vec<BoardObservation>, String> {
    let user = auth.get_user().await?.ok_or("LEARN_AUTH_REQUIRED")?.id;
    history(&local, user, &request)
}

#[tauri::command]
pub async fn learn_observe_board(
    app: AppHandle,
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: ActionRequest,
) -> Result<BoardObservation, String> {
    let (user, course) = environment::verified(&auth, &request.environment).await?;
    authorize(&course, &request)?;
    let (binding, device) =
        environment::bound_board(&local, &workbench, user, &request.environment)?;
    let mut record = BoardObservation {
        observation_id: format!("board_{:032x}", rand::random::<u128>()),
        request,
        project_id: binding.project_id,
        project_name: binding.project_name,
        board_uid: binding.board_uid,
        board_name: binding.board_name,
        device_id: device.id.ok_or("LEARN_BOARD_MISSING")?,
        binding_digest: binding.config_digest,
        state: ObservationState::Unknown,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        snapshot: None,
        evidence_sha256: None,
        error_code: None,
        sync_status: LocalSync::LocalOnly,
        trust: ObservationTrust::ClientObservation,
    };
    persist(&local, user, &record, true)?;
    let result = workbench::linux_overview::saved_device_overview(
        device,
        app.clone(),
        &app.state::<crate::adb::commands::AdbState>(),
        app.state::<Arc<crate::serial::SerialState>>().inner(),
        workbench::linux_overview::OverviewAccess::PassiveBound,
    )
    .await;
    let account = auth.get_user().await;
    let same_user = account
        .as_ref()
        .ok()
        .and_then(|u| u.as_ref())
        .is_some_and(|u| u.id == user);
    let current = same_user
        && environment::bound(&local, &workbench, user, &record.request.environment)
            .is_ok_and(|v| v.0.config_digest == record.binding_digest);
    finish(&mut record, result, current)?;
    persist(&local, user, &record, false)?;
    if account.is_err() {
        return Err("LEARN_AUTH_STATE_UNAVAILABLE".into());
    }
    if !same_user {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (manifest::VerifiedCourse, ActionRequest) {
        let mut course = manifest::tests::verified();
        course.missing_capabilities.clear();
        course.manifest.schema_version = 2;
        let unit = &course.manifest.chapters[0].units[0];
        let activity = unit
            .activities
            .iter()
            .find(|a| !a.action_refs.is_empty())
            .unwrap();
        let request = ActionRequest {
            environment: super::super::environment::EnvironmentRequest {
                enrollment_id: course.enrollment_id.clone(),
                release_id: course.manifest.release_id.clone(),
                manifest_sha256: course.manifest_sha256.clone(),
                profile_id: activity.environment_ref.clone(),
            },
            unit_id: unit.unit_id.clone(),
            activity_id: activity.activity_id.clone(),
            action_id: activity.action_refs[0].clone(),
        };
        let action = course
            .manifest
            .actions
            .iter_mut()
            .find(|a| a.action_id == request.action_id)
            .unwrap();
        action.capability_id = Some("learn.board.observe".into());
        action.catalog_version = "1.0.0".into();
        action.target = "board".into();
        action.risk = "read_only".into();
        action.parameters.clear();
        action.evidence_outputs = vec!["board_observation".into()];
        course
            .manifest
            .environments
            .iter_mut()
            .find(|p| p.profile_id == request.environment.profile_id)
            .unwrap()
            .target = "board".into();
        (course, request)
    }

    #[test]
    fn board_observation_requires_exact_signed_scope_and_no_target_override() {
        let (course, request) = fixture();
        assert!(authorize(&course, &request).is_ok());
        for field in [
            "enrollmentId",
            "releaseId",
            "manifestSha256",
            "profileId",
            "unitId",
            "activityId",
            "actionId",
        ] {
            let mut value = serde_json::to_value(&request).unwrap();
            value[field] = json!("wrong");
            assert!(
                authorize(&course, &serde_json::from_value(value).unwrap()).is_err(),
                "accepted wrong {field}"
            );
        }
        for field in ["deviceId", "serialPort", "adbSerial", "command", "shell"] {
            let mut invalid = fixture().0;
            invalid
                .manifest
                .actions
                .iter_mut()
                .find(|a| a.action_id == request.action_id)
                .unwrap()
                .parameters
                .insert(field.into(), json!("override"));
            assert!(authorize(&invalid, &request).is_err());
        }
        let mut invalid = fixture().0;
        invalid.missing_capabilities.push("unknown.required".into());
        assert!(authorize(&invalid, &request).is_err());
        for (field, replacement) in [
            ("capabilityId", json!("mcu.memory.read")),
            ("catalogVersion", json!("2.0.0")),
            ("target", json!("ubuntu")),
            ("risk", json!("device_write")),
            ("evidenceOutputs", json!(["server_verified"])),
        ] {
            let mut invalid = fixture().0;
            let action = invalid
                .manifest
                .actions
                .iter_mut()
                .find(|a| a.action_id == request.action_id)
                .unwrap();
            let mut value = serde_json::to_value(&*action).unwrap();
            value[field] = replacement;
            *action = serde_json::from_value(value).unwrap();
            assert!(authorize(&invalid, &request).is_err(), "accepted {field}");
        }
        let mut forged = serde_json::to_value(&request).unwrap();
        forged["deviceId"] = json!(8);
        assert!(serde_json::from_value::<ActionRequest>(forged).is_err());
        let mut invalid = fixture().0;
        invalid.manifest.schema_version = 1;
        assert!(authorize(&invalid, &request).is_err());
        let mut invalid = fixture().0;
        invalid
            .manifest
            .environments
            .iter_mut()
            .find(|p| p.profile_id == request.environment.profile_id)
            .unwrap()
            .target = "ubuntu".into();
        assert!(authorize(&invalid, &request).is_err());
    }

    fn record() -> BoardObservation {
        BoardObservation {
            observation_id: "board_test".into(),
            request: fixture().1,
            project_id: 1,
            project_name: "test project".into(),
            board_uid: "linux-test".into(),
            board_name: "test board".into(),
            device_id: 7,
            binding_digest: "b".repeat(64),
            state: ObservationState::Unknown,
            started_at: "before".into(),
            finished_at: None,
            snapshot: None,
            evidence_sha256: None,
            error_code: None,
            sync_status: LocalSync::LocalOnly,
            trust: ObservationTrust::ClientObservation,
        }
    }
    fn observation() -> workbench::linux_overview::LinuxDeviceOverview {
        let mut value = workbench::linux_overview::LinuxDeviceOverview::default();
        value.device_id = 7;
        value.transport = "serial".into();
        value.collected_at_ms = 123;
        value.architecture = "armv7l".into();
        value.kernel = "test kernel".into();
        value.os_name = "Linux\npassword=canary-password".into();
        value.adb_serial = "canary-adb".into();
        value.ip_addresses = vec!["canary-private-address".into()];
        value
    }
    #[test]
    fn observation_is_local_redacted_and_stale_after_rebinding() {
        let mut value = record();
        finish(&mut value, Ok(observation()), false).unwrap();
        assert_eq!(value.state, ObservationState::Stale);
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(!encoded.contains("canary"));
        assert!(encoded.contains("armv7l"));
        assert_eq!(value.evidence_sha256.as_ref().unwrap().len(), 64);
        assert!(encoded.contains("local_only"));
        assert!(encoded.contains("client_observation"));
        let mut forged = serde_json::to_value(&value).unwrap();
        forged["syncStatus"] = json!("accepted");
        assert!(serde_json::from_value::<BoardObservation>(forged).is_err());
        let mut failed = record();
        finish(
            &mut failed,
            Err("SERIAL_OVERVIEW_NOT_OPEN:canary-private-path".into()),
            true,
        )
        .unwrap();
        assert_eq!(
            failed.error_code.as_deref(),
            Some("SERIAL_OVERVIEW_NOT_OPEN")
        );
        assert!(failed.snapshot.is_none());
        assert!(failed.evidence_sha256.is_none());
        let mut wrong = observation();
        wrong.device_id = 8;
        let mut failed = record();
        finish(&mut failed, Ok(wrong), true).unwrap();
        assert_eq!(failed.state, ObservationState::Failed);
        assert!(failed.snapshot.is_none());
    }
    #[test]
    fn interrupted_observation_survives_reopen_and_history_never_crosses_owner_or_scope() {
        let path = std::env::temp_dir().join(format!(
            "lynx-board-observation-test-{:032x}.sqlite",
            rand::random::<u128>()
        ));
        let mut value = record();
        {
            let local = LocalLearningState::open(&path, std::env::temp_dir()).unwrap();
            persist(&local, 7, &value, true).unwrap();
            assert!(persist(&local, 7, &value, true).is_err());
        }
        {
            let local = LocalLearningState::open(&path, std::env::temp_dir()).unwrap();
            assert_eq!(
                history(&local, 7, &value.request).unwrap()[0].state,
                ObservationState::Unknown
            );
            assert!(history(&local, 8, &value.request).unwrap().is_empty());
            for field in [
                "enrollmentId",
                "releaseId",
                "manifestSha256",
                "profileId",
                "unitId",
                "activityId",
                "actionId",
            ] {
                let mut request = serde_json::to_value(&value.request).unwrap();
                request[field] = json!("different");
                assert!(
                    history(&local, 7, &serde_json::from_value(request).unwrap())
                        .unwrap()
                        .is_empty(),
                    "mixed {field}"
                );
            }
            finish(&mut value, Ok(observation()), true).unwrap();
            assert!(persist(&local, 8, &value, false).is_err());
            persist(&local, 7, &value, false).unwrap();
            assert!(persist(&local, 7, &value, false).is_err());
            assert_eq!(
                history(&local, 7, &value.request).unwrap()[0].state,
                ObservationState::Observed
            );
            for index in 0..51 {
                let mut unrelated = record();
                unrelated.observation_id = format!("other_{index}");
                unrelated.started_at = "later".into();
                unrelated.request.action_id = "other_action".into();
                persist(&local, 7, &unrelated, true).unwrap();
            }
            assert_eq!(
                history(&local, 7, &value.request).unwrap().len(),
                1,
                "other actions displaced this action's history"
            );
            let mut corrupted = serde_json::to_value(&value).unwrap();
            corrupted["snapshot"]["kernel"] = json!("tampered");
            local
                .db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE learn_board_observations SET record=?1 WHERE observation_id=?2",
                    params![corrupted.to_string(), value.observation_id],
                )
                .unwrap();
            assert!(
                history(&local, 7, &value.request).is_err(),
                "corrupted observation was accepted"
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}

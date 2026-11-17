use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::{command, State};

use super::storage;
use super::types::*;
use super::RecordCenterState;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{:016x}",
        now_ms(),
        rand::thread_rng().next_u64()
    )
}

fn normalized_actor_id(actor_id: Option<String>) -> String {
    actor_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local-user".to_owned())
}

fn insert_audit_event(
    db: &rusqlite::Connection,
    actor_id: &str,
    action: &str,
    result: &str,
    error_code: Option<&str>,
    summary: &str,
    metadata: serde_json::Value,
) -> Result<(), String> {
    let event = RecordEvent {
        event_id: new_id("evt"),
        occurred_at_ms: now_ms(),
        actor_type: "user".to_owned(),
        actor_id: actor_id.to_owned(),
        category: "record_center".to_owned(),
        action: action.to_owned(),
        scope_type: "application".to_owned(),
        scope_id: "record-center".to_owned(),
        board_uid: String::new(),
        session_id: None,
        correlation_id: new_id("record-operation"),
        causation_id: None,
        result: result.to_owned(),
        error_code: error_code.map(str::to_owned),
        summary: summary.to_owned(),
        metadata,
    };
    storage::insert_event(db, &event).map_err(|error| error.to_string())
}

pub(crate) fn reject_sensitive(summary: &str, metadata: &serde_json::Value) -> Result<(), String> {
    fn has_sensitive_key(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
                let key = key.to_ascii_lowercase().replace(['-', ' ', '_'], "");
                matches!(
                    key.as_str(),
                    "apikey"
                        | "password"
                        | "privatekey"
                        | "accesstoken"
                        | "refreshtoken"
                        | "authorization"
                        | "secret"
                ) || has_sensitive_key(value)
            }),
            serde_json::Value::Array(values) => values.iter().any(has_sensitive_key),
            _ => false,
        }
    }
    let combined = format!("{summary}\n{metadata}");
    let secret = Regex::new(r"(?i)(bearer\s+[a-z0-9._~-]{12,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|\b(sk|ghp|xox[baprs])-[a-z0-9_-]{12,}|(api[_ -]?key|password|access[_ -]?token|refresh[_ -]?token)\s*[:=]\s*\S+)").expect("constant regex");
    if has_sensitive_key(metadata) || secret.is_match(&combined) {
        return Err("RECORD_SENSITIVE_CONTENT_REJECTED".to_owned());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    schema_version: u32,
    content_sha256: String,
    contains_sensitive_bodies: bool,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupRecords {
    events: Vec<RecordEvent>,
    sessions: Vec<RecordSession>,
    feedback_drafts: Vec<FeedbackDraft>,
}

#[derive(Debug, Deserialize)]
struct BackupEnvelope {
    manifest: BackupManifest,
    records: BackupRecords,
}

fn load_backup(path: &str) -> Result<BackupEnvelope, String> {
    let path = PathBuf::from(path);
    let metadata = std::fs::metadata(&path).map_err(|_| "RESTORE_BACKUP_NOT_FOUND".to_owned())?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        return Err("RESTORE_BACKUP_SIZE_INVALID".to_owned());
    }
    let bytes = std::fs::read(&path).map_err(|_| "RESTORE_BACKUP_READ_FAILED".to_owned())?;
    let raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "RESTORE_BACKUP_SCHEMA_INVALID".to_owned())?;
    let records_bytes = serde_json::to_vec(
        raw.get("records")
            .ok_or_else(|| "RESTORE_BACKUP_SCHEMA_INVALID".to_owned())?,
    )
    .map_err(|_| "RESTORE_BACKUP_SCHEMA_INVALID".to_owned())?;
    let envelope: BackupEnvelope =
        serde_json::from_value(raw).map_err(|_| "RESTORE_BACKUP_SCHEMA_INVALID".to_owned())?;
    if !matches!(envelope.manifest.schema_version, 1 | 2)
        || envelope.manifest.contains_sensitive_bodies
    {
        return Err("RESTORE_BACKUP_POLICY_INCOMPATIBLE".to_owned());
    }
    let actual_hash = hex::encode(Sha256::digest(&records_bytes));
    if actual_hash != envelope.manifest.content_sha256 {
        return Err("RESTORE_BACKUP_HASH_MISMATCH".to_owned());
    }
    for event in &envelope.records.events {
        reject_sensitive(&event.summary, &event.metadata)?;
    }
    for draft in &envelope.records.feedback_drafts {
        if draft.include_sensitive_body {
            return Err("RESTORE_BACKUP_POLICY_INCOMPATIBLE".to_owned());
        }
        reject_sensitive(
            &format!("{}\n{}\n{}", draft.title, draft.expected, draft.actual),
            &json!({}),
        )?;
    }
    Ok(envelope)
}

#[command]
pub fn record_center_append_event(
    request: AppendEventRequest,
    state: State<'_, Arc<RecordCenterState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
) -> Result<RecordEvent, String> {
    reject_sensitive(&request.summary, &request.metadata)?;
    if request.actor_type.trim().is_empty()
        || request.action.trim().is_empty()
        || request.summary.trim().is_empty()
    {
        return Err("RECORD_REQUIRED_FIELD_MISSING".to_owned());
    }
    let event_id = new_id("evt");
    let occurred_at_ms = now_ms();
    let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    storage::sync_board_identities(&mut db, &workbench_db).map_err(|error| error.to_string())?;
    drop(workbench_db);
    let scope_id = request.scope_id.trim().to_owned();
    let event = RecordEvent {
        event_id: event_id.clone(),
        occurred_at_ms,
        actor_type: request.actor_type.trim().to_owned(),
        actor_id: request.actor_id.trim().to_owned(),
        category: request.category.trim().to_owned(),
        action: request.action.trim().to_owned(),
        scope_type: request.scope_type.trim().to_owned(),
        board_uid: storage::board_uid_for_scope(&db, &scope_id)
            .map_err(|error| error.to_string())?,
        scope_id,
        session_id: request.session_id.clone(),
        correlation_id: request.correlation_id.unwrap_or_else(|| event_id.clone()),
        causation_id: request.causation_id,
        result: request.result.trim().to_owned(),
        error_code: request.error_code.filter(|value| !value.trim().is_empty()),
        summary: request.summary.trim().to_owned(),
        metadata: request.metadata,
    };
    if let Some(session_id) = &event.session_id {
        storage::upsert_session(
            &db,
            storage::SessionUpsert {
                session_id,
                kind: request.session_kind.as_deref().unwrap_or("other"),
                title: request.session_title.as_deref().unwrap_or(session_id),
                scope_type: &event.scope_type,
                scope_id: &event.scope_id,
                board_uid: &event.board_uid,
                started_at_ms: occurred_at_ms,
            },
        )
        .map_err(|error| error.to_string())?;
    }
    storage::insert_event(&db, &event).map_err(|error| error.to_string())?;
    if request.end_session {
        if let Some(session_id) = &event.session_id {
            storage::end_session(&db, session_id, occurred_at_ms)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(event)
}

#[command]
pub fn record_center_list_events(
    filter: Option<EventFilter>,
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<Vec<RecordEvent>, String> {
    let filter = filter.unwrap_or_default();
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut events = storage::list_events(&db, filter.limit.unwrap_or(200).clamp(1, 500))
        .map_err(|error| error.to_string())?;
    let query = filter.query.unwrap_or_default().to_ascii_lowercase();
    events.retain(|event| {
        (query.is_empty()
            || format!(
                "{} {} {} {} {} {} {}",
                event.summary,
                event.actor_id,
                event.scope_id,
                event.action,
                event.error_code.as_deref().unwrap_or(""),
                event.event_id,
                event.session_id.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase()
            .contains(&query))
            && filter
                .actor_type
                .as_ref()
                .is_none_or(|value| value.is_empty() || &event.actor_type == value)
            && filter
                .scope_id
                .as_ref()
                .is_none_or(|value| value.is_empty() || event.scope_id.contains(value))
            && filter
                .result
                .as_ref()
                .is_none_or(|value| value.is_empty() || &event.result == value)
            && filter.error_code.as_ref().is_none_or(|value| {
                value.is_empty()
                    || event
                        .error_code
                        .as_deref()
                        .is_some_and(|code| code.contains(value))
            })
            && filter
                .session_id
                .as_ref()
                .is_none_or(|value| value.is_empty() || event.session_id.as_ref() == Some(value))
    });
    Ok(events)
}

#[command]
pub fn record_center_overview(
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<RecordOverview, String> {
    let now = now_ms();
    let day_start = now - now.rem_euclid(86_400_000);
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::overview(&db, day_start).map_err(|error| error.to_string())
}

#[command]
pub fn record_center_list_sessions(
    limit: Option<u32>,
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<Vec<RecordSession>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_sessions(&db, limit.unwrap_or(100).clamp(1, 500))
        .map_err(|error| error.to_string())
}

#[command]
pub fn record_center_save_feedback_draft(
    request: SaveFeedbackDraftRequest,
    actor_id: Option<String>,
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<FeedbackDraft, String> {
    if request.title.trim().is_empty() {
        return Err("FEEDBACK_TITLE_REQUIRED".to_owned());
    }
    if request.include_sensitive_body {
        return Err("FEEDBACK_SENSITIVE_BODY_REQUIRES_FUTURE_EXPLICIT_EVIDENCE_FLOW".to_owned());
    }
    reject_sensitive(
        &format!(
            "{}\n{}\n{}",
            request.title, request.expected, request.actual
        ),
        &json!({}),
    )?;
    let actor_id = normalized_actor_id(actor_id);
    let id = request.id.clone().unwrap_or_else(|| new_id("fb"));
    let now = now_ms();
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::save_feedback(&db, &request, &id, now).map_err(|error| error.to_string())?;
    let event = RecordEvent {
        event_id: new_id("evt"),
        occurred_at_ms: now,
        actor_type: "user".to_owned(),
        actor_id,
        category: "feedback".to_owned(),
        action: "feedback.draft_saved".to_owned(),
        scope_type: "application".to_owned(),
        scope_id: "record-center".to_owned(),
        board_uid: String::new(),
        session_id: None,
        correlation_id: id.clone(),
        causation_id: request.event_ids.first().cloned(),
        result: "success".to_owned(),
        error_code: None,
        summary: "保存本地问题反馈草稿".to_owned(),
        metadata: json!({
            "feedbackId": id,
            "problemType": request.problem_type,
            "evidenceEventCount": request.event_ids.len(),
            "containsSensitiveBody": false,
            "uploaded": false,
        }),
    };
    storage::insert_event(&db, &event).map_err(|error| error.to_string())?;
    storage::list_feedback(&db)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "FEEDBACK_SAVE_FAILED".to_owned())
}

#[command]
pub fn record_center_list_feedback_drafts(
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<Vec<FeedbackDraft>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_feedback(&db).map_err(|error| error.to_string())
}

#[command]
pub fn record_center_create_backup(
    target_dir: String,
    actor_id: Option<String>,
    state: State<'_, Arc<RecordCenterState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
) -> Result<BackupRecord, String> {
    let directory = PathBuf::from(target_dir);
    if !directory.is_dir() {
        return Err("BACKUP_TARGET_DIRECTORY_REQUIRED".to_owned());
    }
    let actor_id = normalized_actor_id(actor_id);
    let now = now_ms();
    let id = new_id("backup");
    let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    storage::sync_board_identities(&mut db, &workbench_db).map_err(|error| error.to_string())?;
    drop(workbench_db);
    let events = storage::list_events(&db, 500_000).map_err(|error| error.to_string())?;
    let sessions = storage::list_sessions(&db, 500_000).map_err(|error| error.to_string())?;
    let feedback = storage::list_feedback(&db).map_err(|error| error.to_string())?;
    let records = json!({"events": events, "sessions": sessions, "feedbackDrafts": feedback});
    let records_bytes = serde_json::to_vec(&records).map_err(|error| error.to_string())?;
    let content_sha256 = hex::encode(Sha256::digest(&records_bytes));
    let payload = json!({"manifest":{"schemaVersion":2,"createdAtMs":now,"eventCount":events.len(),"sessionCount":sessions.len(),"feedbackCount":feedback.len(),"contentSha256":content_sha256,"containsSensitiveBodies":false},"records":records});
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?;
    let file_sha256 = hex::encode(Sha256::digest(&bytes));
    let path = directory.join(format!("LYNX-records-{now}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("BACKUP_CREATE_FAILED:{error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("BACKUP_WRITE_FAILED:{error}"))?;
    let record = BackupRecord {
        id,
        path: path.to_string_lossy().into_owned(),
        sha256: file_sha256,
        event_count: events.len() as u32,
        session_count: sessions.len() as u32,
        feedback_count: feedback.len() as u32,
        created_at_ms: now,
    };
    storage::insert_backup(&db, &record).map_err(|error| error.to_string())?;
    let event = RecordEvent {
        event_id: new_id("evt"),
        occurred_at_ms: now,
        actor_type: "user".to_owned(),
        actor_id,
        category: "backup".to_owned(),
        action: "backup.created".to_owned(),
        scope_type: "application".to_owned(),
        scope_id: "record-center".to_owned(),
        board_uid: String::new(),
        session_id: None,
        correlation_id: record.id.clone(),
        causation_id: None,
        result: "success".to_owned(),
        error_code: None,
        summary: "创建本地记录备份".to_owned(),
        metadata: json!({
            "eventCount": record.event_count,
            "sessionCount": record.session_count,
            "feedbackCount": record.feedback_count,
            "sha256": record.sha256,
        }),
    };
    storage::insert_event(&db, &event).map_err(|error| error.to_string())?;
    Ok(record)
}

#[command]
pub fn record_center_list_backups(
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<Vec<BackupRecord>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_backups(&db).map_err(|error| error.to_string())
}

#[command]
pub fn record_center_preview_restore(
    source_path: String,
    actor_id: Option<String>,
    state: State<'_, Arc<RecordCenterState>>,
) -> Result<RestorePreview, String> {
    let actor_id = normalized_actor_id(actor_id);
    let envelope = match load_backup(&source_path) {
        Ok(envelope) => envelope,
        Err(error) => {
            let db = state
                .db
                .lock()
                .map_err(|lock_error| lock_error.to_string())?;
            insert_audit_event(
                &db,
                &actor_id,
                "backup.restore_preview_failed",
                "failed",
                Some(&error),
                "本地记录备份恢复预检失败",
                json!({"sourcePathRecorded": false}),
            )?;
            return Err(error);
        }
    };
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let event_ids = storage::existing_ids(&db, "record_event", "event_id")
        .map_err(|error| error.to_string())?;
    let session_ids = storage::existing_ids(&db, "record_session", "session_id")
        .map_err(|error| error.to_string())?;
    let feedback_ids =
        storage::existing_ids(&db, "feedback_draft", "id").map_err(|error| error.to_string())?;
    let conflicting_events = envelope
        .records
        .events
        .iter()
        .filter(|item| event_ids.contains(&item.event_id))
        .count() as u32;
    let conflicting_sessions = envelope
        .records
        .sessions
        .iter()
        .filter(|item| session_ids.contains(&item.session_id))
        .count() as u32;
    let conflicting_feedback = envelope
        .records
        .feedback_drafts
        .iter()
        .filter(|item| feedback_ids.contains(&item.id))
        .count() as u32;
    let preview = RestorePreview {
        path: source_path,
        schema_version: envelope.manifest.schema_version,
        new_events: envelope.records.events.len() as u32 - conflicting_events,
        conflicting_events,
        new_sessions: envelope.records.sessions.len() as u32 - conflicting_sessions,
        conflicting_sessions,
        new_feedback: envelope.records.feedback_drafts.len() as u32 - conflicting_feedback,
        conflicting_feedback,
        content_sha256: envelope.manifest.content_sha256,
    };
    insert_audit_event(
        &db,
        &actor_id,
        "backup.restore_previewed",
        "success",
        None,
        "预览本地记录备份恢复范围",
        json!({
            "newEvents": preview.new_events,
            "conflictingEvents": preview.conflicting_events,
            "newSessions": preview.new_sessions,
            "conflictingSessions": preview.conflicting_sessions,
            "newFeedback": preview.new_feedback,
            "conflictingFeedback": preview.conflicting_feedback,
            "contentSha256": preview.content_sha256,
        }),
    )?;
    Ok(preview)
}

#[command]
pub fn record_center_restore_backup(
    source_path: String,
    actor_id: Option<String>,
    state: State<'_, Arc<RecordCenterState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
) -> Result<RestoreResult, String> {
    let actor_id = normalized_actor_id(actor_id);
    let envelope = match load_backup(&source_path) {
        Ok(envelope) => envelope,
        Err(error) => {
            let db = state
                .db
                .lock()
                .map_err(|lock_error| lock_error.to_string())?;
            insert_audit_event(
                &db,
                &actor_id,
                "backup.restore_failed",
                "failed",
                Some(&error),
                "本地记录备份恢复失败",
                json!({"sourcePathRecorded": false}),
            )?;
            return Err(error);
        }
    };
    let total = envelope.records.events.len()
        + envelope.records.sessions.len()
        + envelope.records.feedback_drafts.len();
    let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    storage::sync_board_identities(&mut db, &workbench_db).map_err(|error| error.to_string())?;
    storage::validate_restored_board_identities(
        &db,
        &envelope.records.events,
        &envelope.records.sessions,
    )
    .map_err(|error| error.to_string())?;
    drop(workbench_db);
    let (imported_events, imported_sessions, imported_feedback) = storage::restore_non_overwrite(
        &mut db,
        &envelope.records.events,
        &envelope.records.sessions,
        &envelope.records.feedback_drafts,
    )
    .map_err(|error| error.to_string())?;
    let imported = imported_events + imported_sessions + imported_feedback;
    let result = RestoreResult {
        imported_events,
        imported_sessions,
        imported_feedback,
        skipped_conflicts: total as u32 - imported,
    };
    let now = now_ms();
    let event = RecordEvent {
        event_id: new_id("evt"),
        occurred_at_ms: now,
        actor_type: "user".to_owned(),
        actor_id,
        category: "backup".to_owned(),
        action: "backup.restored_non_overwrite".to_owned(),
        scope_type: "application".to_owned(),
        scope_id: "record-center".to_owned(),
        board_uid: String::new(),
        session_id: None,
        correlation_id: new_id("restore"),
        causation_id: None,
        result: "success".to_owned(),
        error_code: None,
        summary: "非覆盖恢复本地记录备份".to_owned(),
        metadata: json!({
            "importedEvents": result.imported_events,
            "importedSessions": result.imported_sessions,
            "importedFeedback": result.imported_feedback,
            "skippedConflicts": result.skipped_conflicts,
            "contentSha256": envelope.manifest.content_sha256,
        }),
    };
    storage::insert_event(&db, &event).map_err(|error| error.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_are_rejected_without_echoing_them() {
        assert_eq!(
            reject_sensitive("saved", &json!({"apiKey":"do-not-store"})).unwrap_err(),
            "RECORD_SENSITIVE_CONTENT_REJECTED"
        );
        assert!(reject_sensitive("connected", &json!({"port":"COM7"})).is_ok());
        assert!(reject_sensitive("Authorization: Bearer abcdefghijklmnop", &json!({})).is_err());
    }

    #[test]
    fn backup_loader_verifies_content_hash() {
        let records = json!({"events":[], "sessions":[], "feedbackDrafts":[]});
        let content_hash = hex::encode(Sha256::digest(serde_json::to_vec(&records).unwrap()));
        let payload = json!({
            "manifest": {
                "schemaVersion": 1,
                "contentSha256": content_hash,
                "containsSensitiveBodies": false
            },
            "records": records
        });
        let path = std::env::temp_dir().join(format!("lynx-record-test-{}.json", new_id("backup")));
        std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();

        let loaded = load_backup(path.to_str().unwrap()).unwrap();
        assert!(loaded.records.events.is_empty());

        let mut tampered = payload;
        tampered["manifest"]["contentSha256"] = json!("0".repeat(64));
        std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert_eq!(
            load_backup(path.to_str().unwrap()).unwrap_err(),
            "RESTORE_BACKUP_HASH_MISMATCH"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn backup_loader_accepts_v1_without_board_uid_and_v2_with_board_uid() {
        for (schema_version, board_uid) in [
            (1, None),
            (2, Some("linux-55555555555555555555555555555555")),
        ] {
            let mut event = json!({
                "eventId":"evt-compat",
                "occurredAtMs":1,
                "actorType":"device",
                "actorId":"linux:1",
                "category":"device",
                "action":"device.state",
                "scopeType":"board",
                "scopeId":"linux:1",
                "sessionId":null,
                "correlationId":"corr-compat",
                "causationId":null,
                "result":"success",
                "errorCode":null,
                "summary":"compatible",
                "metadata":{}
            });
            if let Some(board_uid) = board_uid {
                event["boardUid"] = json!(board_uid);
            }
            let records = json!({"events":[event], "sessions":[], "feedbackDrafts":[]});
            let content_hash = hex::encode(Sha256::digest(serde_json::to_vec(&records).unwrap()));
            let payload = json!({
                "manifest": {
                    "schemaVersion": schema_version,
                    "contentSha256": content_hash,
                    "containsSensitiveBodies": false
                },
                "records": records
            });
            let path = std::env::temp_dir().join(format!(
                "lynx-record-compat-{schema_version}-{}.json",
                new_id("backup")
            ));
            std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
            let loaded = load_backup(path.to_str().unwrap()).unwrap();
            assert_eq!(
                loaded.records.events[0].board_uid,
                board_uid.unwrap_or_default()
            );
            std::fs::remove_file(path).unwrap();
        }
    }
}

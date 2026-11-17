//! Local-first record center.
//!
//! This module is business-neutral: Linux, MCU, AI and UI producers submit the
//! same event envelope, while credentials and hardware capabilities stay in
//! their owning domains.

pub mod commands;
pub(crate) mod storage;
pub mod types;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use rusqlite::Connection;
use tauri::Manager;

use crate::audit::{AuditEventDraft, AuditSink};

pub struct RecordCenterState {
    pub(crate) db: Mutex<Connection>,
}

impl RecordCenterState {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        storage::migrate(&connection).map_err(|error| error.to_string())?;
        Ok(Self {
            db: Mutex::new(connection),
        })
    }
}

struct RecordCenterAuditAdapter {
    records: Arc<RecordCenterState>,
    workbench: Arc<crate::workbench::WorkbenchState>,
}

impl AuditSink for RecordCenterAuditAdapter {
    fn append(&self, draft: AuditEventDraft) -> Result<String, String> {
        commands::reject_sensitive(&draft.summary, &draft.metadata)?;
        validate_draft(&draft)?;

        let workbench_db = self
            .workbench
            .db
            .lock()
            .map_err(|_| "AUDIT_WORKBENCH_STORE_UNAVAILABLE".to_owned())?;
        let mut records_db = self
            .records
            .db
            .lock()
            .map_err(|_| "AUDIT_RECORD_STORE_UNAVAILABLE".to_owned())?;
        storage::sync_board_identities(&mut records_db, &workbench_db)
            .map_err(|_| "AUDIT_BOARD_IDENTITY_SYNC_FAILED".to_owned())?;

        let event_id = new_id("evt");
        let occurred_at_ms = now_ms();
        let event = types::RecordEvent {
            event_id: event_id.clone(),
            occurred_at_ms,
            actor_type: draft.actor_type.trim().to_owned(),
            actor_id: draft.actor_id.trim().to_owned(),
            category: draft.category.trim().to_owned(),
            action: draft.action.trim().to_owned(),
            scope_type: draft.scope_type.trim().to_owned(),
            board_uid: storage::board_uid_for_scope(&records_db, draft.scope_id.trim())
                .map_err(|_| "AUDIT_BOARD_IDENTITY_LOOKUP_FAILED".to_owned())?,
            scope_id: draft.scope_id.trim().to_owned(),
            session_id: draft.session_id,
            correlation_id: draft.correlation_id.unwrap_or_else(|| event_id.clone()),
            causation_id: draft.causation_id,
            result: draft.result.trim().to_owned(),
            error_code: draft.error_code.filter(|value| !value.trim().is_empty()),
            summary: draft.summary.trim().to_owned(),
            metadata: draft.metadata,
        };
        if let Some(session_id) = &event.session_id {
            storage::upsert_session(
                &records_db,
                storage::SessionUpsert {
                    session_id,
                    kind: draft.session_kind.as_deref().unwrap_or("other"),
                    title: draft.session_title.as_deref().unwrap_or(session_id),
                    scope_type: &event.scope_type,
                    scope_id: &event.scope_id,
                    board_uid: &event.board_uid,
                    started_at_ms: occurred_at_ms,
                },
            )
            .map_err(|_| "AUDIT_SESSION_UPSERT_FAILED".to_owned())?;
        }
        storage::insert_event(&records_db, &event)
            .map_err(|_| "AUDIT_EVENT_INSERT_FAILED".to_owned())?;
        if draft.end_session {
            if let Some(session_id) = &event.session_id {
                storage::end_session(&records_db, session_id, occurred_at_ms)
                    .map_err(|_| "AUDIT_SESSION_END_FAILED".to_owned())?;
            }
        }
        Ok(event_id)
    }
}

pub fn register_audit_sink(
    app: &tauri::AppHandle,
    records: Arc<RecordCenterState>,
    workbench: Arc<crate::workbench::WorkbenchState>,
) -> Result<(), &'static str> {
    app.state::<crate::audit::AuditSinkState>()
        .register(Arc::new(RecordCenterAuditAdapter { records, workbench }))
}

fn validate_draft(draft: &AuditEventDraft) -> Result<(), String> {
    if draft.actor_type.trim().is_empty()
        || draft.actor_id.trim().is_empty()
        || draft.category.trim().is_empty()
        || draft.action.trim().is_empty()
        || draft.scope_type.trim().is_empty()
        || draft.scope_id.trim().is_empty()
        || draft.result.trim().is_empty()
        || draft.summary.trim().is_empty()
    {
        return Err("AUDIT_REQUIRED_FIELD_MISSING".to_owned());
    }
    Ok(())
}

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

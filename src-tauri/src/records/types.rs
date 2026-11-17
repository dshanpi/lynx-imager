use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEvent {
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub category: String,
    pub action: String,
    pub scope_type: String,
    pub scope_id: String,
    #[serde(default)]
    pub board_uid: String,
    pub session_id: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
    pub result: String,
    pub error_code: Option<String>,
    pub summary: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendEventRequest {
    pub actor_type: String,
    pub actor_id: String,
    pub category: String,
    pub action: String,
    #[serde(default)]
    pub scope_type: String,
    #[serde(default)]
    pub scope_id: String,
    pub session_id: Option<String>,
    pub session_kind: Option<String>,
    pub session_title: Option<String>,
    #[serde(default)]
    pub end_session: bool,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub result: String,
    pub error_code: Option<String>,
    pub summary: String,
    #[serde(default = "empty_metadata")]
    pub metadata: serde_json::Value,
}

fn empty_metadata() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    pub query: Option<String>,
    pub actor_type: Option<String>,
    pub scope_id: Option<String>,
    pub result: Option<String>,
    pub error_code: Option<String>,
    pub session_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSession {
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub scope_type: String,
    pub scope_id: String,
    #[serde(default)]
    pub board_uid: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub event_count: u32,
    pub integrity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordOverview {
    pub today_event_count: u32,
    pub active_serial_sessions: u32,
    pub ai_session_count: u32,
    pub device_error_count: u32,
    pub pending_feedback_count: u32,
    pub gap_count: u32,
    pub latest_backup_at_ms: Option<i64>,
    pub lan_service_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackDraft {
    pub id: String,
    pub title: String,
    pub problem_type: String,
    pub page: String,
    pub expected: String,
    pub actual: String,
    pub reproducible: String,
    pub status: String,
    pub event_ids: Vec<String>,
    pub include_sensitive_body: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFeedbackDraftRequest {
    pub id: Option<String>,
    pub title: String,
    pub problem_type: String,
    pub page: String,
    pub expected: String,
    pub actual: String,
    pub reproducible: String,
    #[serde(default)]
    pub event_ids: Vec<String>,
    #[serde(default)]
    pub include_sensitive_body: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecord {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub event_count: u32,
    pub session_count: u32,
    pub feedback_count: u32,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub path: String,
    pub schema_version: u32,
    pub new_events: u32,
    pub conflicting_events: u32,
    pub new_sessions: u32,
    pub conflicting_sessions: u32,
    pub new_feedback: u32,
    pub conflicting_feedback: u32,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub imported_events: u32,
    pub imported_sessions: u32,
    pub imported_feedback: u32,
    pub skipped_conflicts: u32,
}

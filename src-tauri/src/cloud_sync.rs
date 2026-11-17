//! Optional, local-first synchronization of redacted LYNX status to 100ask.net.
//!
//! This module is deliberately read-only with respect to Workbench. It snapshots
//! task facts, persists only a bounded latest outbox document, and never changes
//! task lifecycle, hardware state, or credentials.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::StatusCode;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager, State};

use crate::auth::state::AuthState;
use crate::workbench::{self, WorkbenchState};

const DEFAULT_API_BASE: &str = "https://www.100ask.net/api/v1/lynx";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TASKS_PER_BATCH: u32 = 100;
const MAX_BACKOFF_SECONDS: i64 = 15 * 60;

pub struct CloudSyncState {
    db: Mutex<Connection>,
    client: reqwest::Client,
    api_base: String,
    running: AtomicBool,
}

impl CloudSyncState {
    pub fn open(path: &Path) -> Result<Self, String> {
        let api_base =
            std::env::var("LYNX_CLOUD_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_owned());
        Self::open_with_base(path, &api_base)
    }

    fn open_with_base(path: &Path, api_base: &str) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let db = Connection::open(path).map_err(|error| error.to_string())?;
        migrate(&db).map_err(|error| error.to_string())?;
        Ok(Self {
            db: Mutex::new(db),
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|error| error.to_string())?,
            api_base: api_base.trim_end_matches('/').to_owned(),
            running: AtomicBool::new(false),
        })
    }

    fn config(&self) -> Result<SyncConfig, String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        load_config(&db).map_err(|error| error.to_string())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        db.execute(
            "UPDATE cloud_sync_config SET enabled=?1,updated_at=?2 WHERE id=1",
            params![enabled, now_seconds()],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn status(&self) -> Result<CloudSyncStatus, String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        status_from_db(&db).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone)]
struct SyncConfig {
    enabled: bool,
    instance_id: String,
    registered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSyncStatus {
    pub enabled: bool,
    pub instance_id: String,
    pub state: String,
    pub pending: bool,
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskSnapshot {
    task_id: String,
    kind: String,
    status: String,
    phase: String,
    progress_pct: Option<f64>,
    started_at: i64,
    finished_at: Option<i64>,
    exit_code: Option<i32>,
    error_code: Option<String>,
    revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncBundle {
    instance_id: String,
    revision: i64,
    captured_at: i64,
    development_status: String,
    active_task_id: Option<String>,
    running_task_count: usize,
    tasks: Vec<TaskSnapshot>,
}

struct RunningGuard<'a>(&'a AtomicBool);

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let state = app.state::<Arc<CloudSyncState>>().inner().clone();
            let auth = app.state::<Arc<AuthState>>().inner().clone();
            let workbench = app.state::<Arc<WorkbenchState>>().inner().clone();
            if let Err(error_code) = sync_once(&state, &auth, &workbench, false).await {
                log::debug!("LYNX cloud sync deferred: {error_code}");
            }
        }
    });
}

#[tauri::command]
pub fn cloud_sync_get_status(
    state: State<'_, Arc<CloudSyncState>>,
) -> Result<CloudSyncStatus, String> {
    state.status()
}

#[tauri::command]
pub fn cloud_sync_set_enabled(
    enabled: bool,
    state: State<'_, Arc<CloudSyncState>>,
) -> Result<CloudSyncStatus, String> {
    state.set_enabled(enabled)?;
    state.status()
}

#[tauri::command]
pub async fn cloud_sync_sync_now(
    state: State<'_, Arc<CloudSyncState>>,
    auth: State<'_, Arc<AuthState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<CloudSyncStatus, String> {
    sync_once(state.inner(), auth.inner(), workbench.inner(), true).await?;
    state.status()
}

async fn sync_once(
    state: &Arc<CloudSyncState>,
    auth: &Arc<AuthState>,
    workbench: &Arc<WorkbenchState>,
    force: bool,
) -> Result<(), String> {
    let config = state.config()?;
    if !config.enabled {
        return Err("CLOUD_SYNC_DISABLED".to_owned());
    }
    if state.running.swap(true, Ordering::AcqRel) {
        return Err("CLOUD_SYNC_ALREADY_RUNNING".to_owned());
    }
    let _guard = RunningGuard(&state.running);

    if !force && !retry_is_due(state)? {
        return Err("CLOUD_SYNC_BACKOFF".to_owned());
    }
    let token = match auth.get_access_token().await {
        Ok(token) => token,
        Err(error) => {
            let code = auth_failure_code(&error);
            record_failure(state, code)?;
            return Err(code.to_owned());
        }
    };
    let revision = next_revision(state)?;
    let bundle = capture_bundle(&config.instance_id, revision, workbench)?;
    persist_outbox(state, &bundle)?;
    record_attempt(state)?;

    if !config.registered {
        let registration = json!({
            "instanceId": config.instance_id,
            "appVersion": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "capabilities": ["workbench", "ai-supervisor", "linux-mcp", "mcu-mcp"]
        });
        post_json(state, "instances/register", &token, &registration).await?;
        mark_registered(state)?;
    }

    let heartbeat = json!({
        "instanceId": &bundle.instance_id,
        "revision": bundle.revision,
        "clientTime": bundle.captured_at,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "developmentStatus": &bundle.development_status,
        "activeTaskId": &bundle.active_task_id,
        "runningTaskCount": bundle.running_task_count
    });
    post_json(state, "instances/heartbeat", &token, &heartbeat).await?;
    let task_batch = json!({
        "instanceId": &bundle.instance_id,
        "revision": bundle.revision,
        "tasks": bundle.tasks
    });
    post_json(state, "tasks/batch", &token, &task_batch).await?;
    record_success(state, bundle.revision)?;
    Ok(())
}

async fn post_json(
    state: &Arc<CloudSyncState>,
    endpoint: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<(), String> {
    let send = |base: &str| {
        state
            .client
            .post(format!("{base}/{endpoint}"))
            .bearer_auth(token)
            .json(body)
            .send()
    };
    let response = send(&state.api_base)
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE".to_owned());
    let mut response = match response {
        Ok(response) => response,
        Err(code) => {
            record_failure(state, &code)?;
            return Err(code);
        }
    };
    if response.status() == StatusCode::NOT_FOUND {
        if let Some(legacy_base) = legacy_api_base(&state.api_base) {
            response = match send(&legacy_base).await {
                Ok(response) => response,
                Err(_) => {
                    let code = "NETWORK_UNAVAILABLE";
                    record_failure(state, code)?;
                    return Err(code.to_owned());
                }
            };
        }
    }
    if response.status().is_success() {
        return Ok(());
    }
    let code = match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "AUTH_REQUIRED",
        StatusCode::NOT_FOUND => {
            mark_unregistered(state)?;
            "CLOUD_API_NOT_READY"
        }
        StatusCode::TOO_MANY_REQUESTS => "RATE_LIMITED",
        status if status.is_server_error() => "CLOUD_SERVER_ERROR",
        _ => "CLOUD_REQUEST_REJECTED",
    };
    record_failure(state, code)?;
    Err(code.to_owned())
}

fn legacy_api_base(api_base: &str) -> Option<String> {
    api_base
        .strip_suffix("/api/v1/lynx")
        .map(|origin| format!("{origin}/api/lynx"))
}

fn capture_bundle(
    instance_id: &str,
    revision: i64,
    workbench: &WorkbenchState,
) -> Result<SyncBundle, String> {
    let tasks = {
        let db = workbench.db.lock().map_err(|error| error.to_string())?;
        workbench::storage::list_tasks(&db, MAX_TASKS_PER_BATCH)
            .map_err(|error| error.to_string())?
    };
    let running = tasks.iter().find(|task| task.status == "running");
    let development_status = running
        .map(|task| development_status(&task.kind, &task.phase))
        .unwrap_or_else(|| "idle".to_owned());
    let active_task_id = running.map(|task| task.task_id.clone());
    let running_task_count = tasks.iter().filter(|task| task.status == "running").count();
    let tasks = tasks
        .into_iter()
        .map(|task| TaskSnapshot {
            task_id: bounded_identifier(&task.task_id, 128, "unknown-task"),
            kind: bounded_identifier(&task.kind, 32, "other"),
            status: normalized_status(&task.status),
            phase: bounded_identifier(&task.phase, 64, "unknown"),
            progress_pct: task.progress_pct.map(|value| value.clamp(0.0, 100.0)),
            started_at: task.started_at,
            finished_at: task.finished_at,
            exit_code: task.exit_code,
            error_code: sanitized_error_code(&task.error),
            revision,
        })
        .collect();
    Ok(SyncBundle {
        instance_id: instance_id.to_owned(),
        revision,
        captured_at: now_seconds(),
        development_status,
        active_task_id,
        running_task_count,
        tasks,
    })
}

fn development_status(kind: &str, phase: &str) -> String {
    let combined = format!(
        "{} {}",
        kind.to_ascii_lowercase(),
        phase.to_ascii_lowercase()
    );
    if combined.contains("flash") {
        "flashing"
    } else if combined.contains("build") || combined.contains("compile") {
        "building"
    } else if combined.contains("upload")
        || combined.contains("download")
        || combined.contains("transfer")
    {
        "transferring"
    } else if combined.contains("test") || combined.contains("verify") {
        "testing"
    } else if combined.contains("approval") {
        "waiting_approval"
    } else if combined.contains("human") || combined.contains("operator") {
        "waiting_human"
    } else {
        "ai_running"
    }
    .to_owned()
}

fn normalized_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "queued" | "running" | "success" | "failed" | "cancelled" | "timeout" => {
            status.trim().to_ascii_lowercase()
        }
        _ => "unknown".to_owned(),
    }
}

fn bounded_identifier(value: &str, max_len: usize, fallback: &str) -> String {
    let allowed = Regex::new(r"^[A-Za-z0-9_.:-]+$").expect("constant regex");
    let value = value.trim();
    if value.is_empty() || value.len() > max_len || !allowed.is_match(value) {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn sanitized_error_code(error: &str) -> Option<String> {
    let first = error
        .trim()
        .split([':', ' ', '\n', '\r'])
        .next()
        .unwrap_or_default();
    let allowed = Regex::new(r"^[A-Z][A-Z0-9_]{1,63}$").expect("constant regex");
    if allowed.is_match(first) {
        Some(first.to_owned())
    } else if error.trim().is_empty() {
        None
    } else {
        Some("TASK_FAILED".to_owned())
    }
}

fn auth_failure_code(error: &str) -> &'static str {
    if error.starts_with("LINK_AUTH_REQUIRED")
        || error.starts_with("AUTH_TOKEN_EXPIRED")
        || error.contains("HTTP 401")
        || error.contains("HTTP 403")
    {
        "AUTH_REQUIRED"
    } else {
        "NETWORK_UNAVAILABLE"
    }
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cloud_sync_config(
            id INTEGER PRIMARY KEY CHECK(id=1),
            enabled INTEGER NOT NULL DEFAULT 0,
            instance_id TEXT NOT NULL,
            registered INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS cloud_sync_outbox(
            id INTEGER PRIMARY KEY CHECK(id=1),
            revision INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS cloud_sync_meta(
            id INTEGER PRIMARY KEY CHECK(id=1),
            last_attempt_at INTEGER,
            last_success_at INTEGER,
            last_error_code TEXT,
            revision INTEGER NOT NULL DEFAULT 0
         );",
    )?;
    add_column(db, "cloud_sync_meta", "revision INTEGER NOT NULL DEFAULT 0")?;
    let instance_id = format!("lynx-{:032x}", rand::random::<u128>());
    db.execute(
        "INSERT OR IGNORE INTO cloud_sync_config(id,enabled,instance_id,registered,updated_at)
         VALUES(1,0,?1,0,?2)",
        params![instance_id, now_seconds()],
    )?;
    db.execute("INSERT OR IGNORE INTO cloud_sync_meta(id) VALUES(1)", [])?;
    Ok(())
}

fn add_column(db: &Connection, table: &str, definition: &str) -> rusqlite::Result<()> {
    match db.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), []) {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("duplicate column name") => Ok(()),
        Err(error) => Err(error),
    }
}

fn load_config(db: &Connection) -> rusqlite::Result<SyncConfig> {
    db.query_row(
        "SELECT enabled,instance_id,registered FROM cloud_sync_config WHERE id=1",
        [],
        |row| {
            Ok(SyncConfig {
                enabled: row.get(0)?,
                instance_id: row.get(1)?,
                registered: row.get(2)?,
            })
        },
    )
}

fn status_from_db(db: &Connection) -> rusqlite::Result<CloudSyncStatus> {
    let config = load_config(db)?;
    let pending = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cloud_sync_outbox WHERE id=1)",
        [],
        |row| row.get(0),
    )?;
    let (last_attempt_at, last_success_at, last_error_code): (
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = db.query_row(
        "SELECT last_attempt_at,last_success_at,last_error_code FROM cloud_sync_meta WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let state = if !config.enabled {
        "disabled"
    } else if last_error_code.as_deref() == Some("AUTH_REQUIRED") {
        "auth_required"
    } else if pending {
        "pending"
    } else if last_success_at.is_some() {
        "synced"
    } else {
        "ready"
    };
    Ok(CloudSyncStatus {
        enabled: config.enabled,
        instance_id: config.instance_id,
        state: state.to_owned(),
        pending,
        last_attempt_at,
        last_success_at,
        last_error_code,
    })
}

fn persist_outbox(state: &CloudSyncState, bundle: &SyncBundle) -> Result<(), String> {
    let payload = serde_json::to_string(bundle).map_err(|error| error.to_string())?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO cloud_sync_outbox(id,revision,payload_json,attempts,next_attempt_at,updated_at)
         VALUES(1,?1,?2,0,0,?3)
         ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,payload_json=excluded.payload_json,
         updated_at=excluded.updated_at",
        params![bundle.revision, payload, now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn next_revision(state: &CloudSyncState) -> Result<i64, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.query_row(
        "UPDATE cloud_sync_meta SET revision=revision+1 WHERE id=1 RETURNING revision",
        [],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

fn retry_is_due(state: &CloudSyncState) -> Result<bool, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let next: Option<i64> = db
        .query_row(
            "SELECT next_attempt_at FROM cloud_sync_outbox WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(next.is_none_or(|value| value <= now_seconds()))
}

fn record_attempt(state: &CloudSyncState) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE cloud_sync_meta SET last_attempt_at=?1 WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_failure(state: &CloudSyncState, code: &str) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let attempts: i64 = db
        .query_row(
            "SELECT attempts FROM cloud_sync_outbox WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0_i64)
        .saturating_add(1);
    let exponent = attempts.clamp(0, 5) as u32;
    let backoff = (30_i64.saturating_mul(2_i64.pow(exponent))).min(MAX_BACKOFF_SECONDS);
    db.execute(
        "UPDATE cloud_sync_outbox SET attempts=?1,next_attempt_at=?2 WHERE id=1",
        params![attempts, now_seconds().saturating_add(backoff)],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE cloud_sync_meta SET last_attempt_at=?1,last_error_code=?2 WHERE id=1",
        params![
            now_seconds(),
            bounded_identifier(code, 64, "CLOUD_SYNC_FAILED")
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_success(state: &CloudSyncState, revision: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM cloud_sync_outbox WHERE id=1 AND revision=?1",
        [revision],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE cloud_sync_meta SET last_attempt_at=?1,last_success_at=?1,last_error_code=NULL WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn mark_registered(state: &CloudSyncState) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE cloud_sync_config SET registered=1,updated_at=?1 WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn mark_unregistered(state: &CloudSyncState) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE cloud_sync_config SET registered=0,updated_at=?1 WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode as AxumStatusCode, routing::post, Router};
    use rusqlite::Connection;

    fn state() -> CloudSyncState {
        CloudSyncState::open_with_base(Path::new(":memory:"), "https://example.invalid/api/lynx")
            .expect("cloud state")
    }

    #[test]
    fn migration_defaults_to_explicit_opt_in_and_stable_instance() {
        let state = state();
        let first = state.config().unwrap();
        let second = state.config().unwrap();
        assert!(!first.enabled);
        assert_eq!(first.instance_id, second.instance_id);
        assert!(first.instance_id.starts_with("lynx-"));
    }

    #[test]
    fn error_redaction_never_uploads_paths_or_freeform_text() {
        assert_eq!(
            sanitized_error_code("SERIAL_TIMEOUT: /home/user/secret"),
            Some("SERIAL_TIMEOUT".into())
        );
        assert_eq!(
            sanitized_error_code("failed at C:\\private\\firmware.bin"),
            Some("TASK_FAILED".into())
        );
        assert_eq!(sanitized_error_code(""), None);
    }

    #[test]
    fn identifiers_and_statuses_are_strictly_bounded() {
        assert_eq!(bounded_identifier("task-1", 32, "fallback"), "task-1");
        assert_eq!(
            bounded_identifier("/home/user/project", 32, "fallback"),
            "fallback"
        );
        assert_eq!(normalized_status("RUNNING"), "running");
        assert_eq!(normalized_status("invented"), "unknown");
    }

    #[test]
    fn auth_refresh_failures_map_to_stable_codes_without_exposing_error_text() {
        assert_eq!(
            auth_failure_code("LINK_AUTH_REQUIRED:请先登录 100ASK 账号"),
            "AUTH_REQUIRED"
        );
        assert_eq!(
            auth_failure_code("AUTH_TOKEN_EXPIRED:登录已过期，请重新登录"),
            "AUTH_REQUIRED"
        );
        assert_eq!(
            auth_failure_code("刷新登录会话失败: HTTP 401 Unauthorized"),
            "AUTH_REQUIRED"
        );
        assert_eq!(
            auth_failure_code("刷新登录会话请求失败: connection refused at a private URL"),
            "NETWORK_UNAVAILABLE"
        );
    }

    #[test]
    fn outbox_keeps_only_the_latest_snapshot_and_clears_exact_revision() {
        let state = state();
        let bundle = |revision| SyncBundle {
            instance_id: "lynx-test".into(),
            revision,
            captured_at: 1,
            development_status: "idle".into(),
            active_task_id: None,
            running_task_count: 0,
            tasks: vec![],
        };
        persist_outbox(&state, &bundle(1)).unwrap();
        persist_outbox(&state, &bundle(2)).unwrap();
        record_success(&state, 1).unwrap();
        assert!(state.status().unwrap().pending);
        record_success(&state, 2).unwrap();
        assert!(!state.status().unwrap().pending);
    }

    #[test]
    fn backoff_is_bounded_and_records_only_stable_error_code() {
        let state = state();
        persist_outbox(
            &state,
            &SyncBundle {
                instance_id: "lynx-test".into(),
                revision: 1,
                captured_at: 1,
                development_status: "idle".into(),
                active_task_id: None,
                running_task_count: 0,
                tasks: vec![],
            },
        )
        .unwrap();
        for _ in 0..20 {
            record_failure(&state, "NETWORK_UNAVAILABLE").unwrap();
        }
        let db = state.db.lock().unwrap();
        let (attempts, next): (i64, i64) = db
            .query_row(
                "SELECT attempts,next_attempt_at FROM cloud_sync_outbox WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts, 20);
        assert!(next <= now_seconds() + MAX_BACKOFF_SECONDS);
    }

    #[test]
    fn schema_contains_no_token_or_secret_column() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let sql: String = db
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE type='table' AND name LIKE 'cloud_sync_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let normalized = sql.to_ascii_lowercase();
        assert!(!normalized.contains("access_token"));
        assert!(!normalized.contains("refresh_token"));
        assert!(!normalized.contains("secret"));
    }

    async fn mock_state(status: AxumStatusCode) -> Arc<CloudSyncState> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(post(move || async move { status }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Arc::new(
            CloudSyncState::open_with_base(
                Path::new(":memory:"),
                &format!("http://{address}/api/lynx"),
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn http_success_does_not_require_or_parse_a_response_body() {
        let state = mock_state(AxumStatusCode::NO_CONTENT).await;
        post_json(&state, "instances/register", "test-only-token", &json!({}))
            .await
            .unwrap();
        assert!(state.status().unwrap().last_error_code.is_none());
    }

    #[tokio::test]
    async fn http_auth_and_missing_api_fail_with_stable_redacted_codes() {
        let unauthorized = mock_state(AxumStatusCode::UNAUTHORIZED).await;
        assert_eq!(
            post_json(&unauthorized, "tasks/batch", "test-only-token", &json!({})).await,
            Err("AUTH_REQUIRED".into())
        );
        assert_eq!(
            unauthorized.status().unwrap().last_error_code.as_deref(),
            Some("AUTH_REQUIRED")
        );

        let missing = mock_state(AxumStatusCode::NOT_FOUND).await;
        assert_eq!(
            post_json(&missing, "tasks/batch", "test-only-token", &json!({})).await,
            Err("CLOUD_API_NOT_READY".into())
        );
        assert_eq!(
            missing.status().unwrap().last_error_code.as_deref(),
            Some("CLOUD_API_NOT_READY")
        );
    }

    #[tokio::test]
    async fn v1_not_found_falls_back_to_same_origin_legacy_route() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/api/v1/lynx/instances/register",
                post(|| async { AxumStatusCode::NOT_FOUND }),
            )
            .route(
                "/api/lynx/instances/register",
                post(|| async { AxumStatusCode::NO_CONTENT }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let state = Arc::new(
            CloudSyncState::open_with_base(
                Path::new(":memory:"),
                &format!("http://{address}/api/v1/lynx"),
            )
            .unwrap(),
        );

        post_json(&state, "instances/register", "test-only-token", &json!({}))
            .await
            .unwrap();
        assert!(state.status().unwrap().last_error_code.is_none());
    }

    #[tokio::test]
    async fn v1_auth_rejection_never_falls_back_to_legacy_route() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/api/v1/lynx/tasks/batch",
                post(|| async { AxumStatusCode::UNAUTHORIZED }),
            )
            .route(
                "/api/lynx/tasks/batch",
                post(|| async { AxumStatusCode::NO_CONTENT }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let state = Arc::new(
            CloudSyncState::open_with_base(
                Path::new(":memory:"),
                &format!("http://{address}/api/v1/lynx"),
            )
            .unwrap(),
        );

        assert_eq!(
            post_json(&state, "tasks/batch", "test-only-token", &json!({})).await,
            Err("AUTH_REQUIRED".into())
        );
    }
}

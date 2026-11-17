//! Local-first software diagnostics for authenticated 100ASK users.
//!
//! The reporter never acquires a hardware guard and never opens or controls a
//! device. It accepts a fixed schema, removes sensitive material before local
//! persistence, and uploads after a valid login unless the user turns it off.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::StatusCode;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};

use crate::auth::state::AuthState;
use crate::hardware_safety;

const DEFAULT_API_BASE: &str = "https://100ask.net/api/v1/lynx/diagnostics";
const REPORT_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(not(test))]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const REQUEST_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_PENDING_REPORTS: usize = 200;
const MAX_BATCH_REPORTS: usize = 20;
const MAX_REPORT_BYTES: usize = 32 * 1024;
const RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_BACKOFF_SECONDS: i64 = 60 * 60;
const SCHEMA_VERSION: i64 = 2;
const DIAGNOSTIC_WIRE_REVISION: i64 = 4;
const MAX_LOCAL_INCIDENTS: usize = 500;
const MAX_RESOLUTION_QUERY: usize = 50;

static PANIC_MARKER_PATH: OnceLock<PathBuf> = OnceLock::new();
static NATIVE_OPERATION_MARKER: OnceLock<Mutex<NativeOperationMarkerRuntime>> = OnceLock::new();

#[derive(Default)]
struct NativeOperationMarkerRuntime {
    path: PathBuf,
    active: Vec<NativeOperationEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeOperationEntry {
    operation_type: String,
    started_at: i64,
}

pub(crate) struct NativeOperationGuard {
    operation_type: String,
}

impl Drop for NativeOperationGuard {
    fn drop(&mut self) {
        let Some(runtime) = NATIVE_OPERATION_MARKER.get() else {
            return;
        };
        let Ok(mut runtime) = runtime.lock() else {
            return;
        };
        if let Some(index) = runtime
            .active
            .iter()
            .rposition(|entry| entry.operation_type == self.operation_type)
        {
            runtime.active.remove(index);
        }
        persist_native_operation_marker(&runtime);
    }
}

pub struct DiagnosticState {
    db: Mutex<Connection>,
    client: reqwest::Client,
    api_base: String,
    running: AtomicBool,
    suspended: AtomicBool,
    logout_epoch: AtomicU64,
    logout_cleanup_pending: AtomicBool,
}

#[derive(Debug, Clone)]
struct DiagnosticConfig {
    enabled: bool,
    instance_id: String,
    user_disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReportingStatus {
    pub enabled: bool,
    pub state: String,
    pub pending_count: usize,
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticCaptureRequest {
    pub source: String,
    pub component: String,
    pub severity: String,
    pub error_code: String,
    pub machine_code: Option<String>,
    pub message_template: Option<String>,
    pub stack: Option<String>,
    pub operation_type: Option<String>,
    pub task_phase: Option<String>,
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub failure_class: Option<String>,
    #[serde(default)]
    pub resolution_key: Option<String>,
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default)]
    pub recommended_skill: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCaptureResult {
    pub report_id: String,
    pub fingerprint: String,
    pub queued: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReportPreview {
    pub report_id: String,
    pub schema_version: i64,
    pub occurred_at: i64,
    pub last_occurred_at: i64,
    pub occurrence_count: i64,
    pub app_version: String,
    pub build_commit: String,
    pub platform: String,
    pub architecture: String,
    pub source: String,
    pub component: String,
    pub severity: String,
    pub error_code: String,
    pub machine_code: Option<String>,
    pub message_template: Option<String>,
    pub stack_fingerprint: Option<String>,
    pub fingerprint: String,
    pub operation_type: Option<String>,
    pub task_phase: Option<String>,
    pub correlation_id: Option<String>,
    pub failure_class: String,
    pub resolution_key: String,
    pub retryable: bool,
    pub recommended_skill: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticIncident {
    pub fingerprint: String,
    pub first_occurred_at: i64,
    pub last_occurred_at: i64,
    pub occurrence_count: i64,
    pub component: String,
    pub severity: String,
    pub error_code: String,
    pub machine_code: Option<String>,
    pub operation_type: Option<String>,
    pub task_phase: Option<String>,
    pub failure_class: String,
    pub resolution_key: String,
    pub retryable: bool,
    pub recommended_skill: Option<String>,
    pub upload_state: String,
    pub dismissed: bool,
    pub resolution_status: Option<String>,
    pub public_workaround: Option<String>,
    pub target_version: Option<String>,
    pub released_version: Option<String>,
    pub resolution_updated_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticResolutionQuery<'a> {
    fingerprints: &'a [String],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticResolution {
    fingerprint: String,
    status: String,
    resolution_key: String,
    public_workaround: Option<String>,
    target_version: Option<String>,
    released_version: Option<String>,
    recommended_skill: Option<String>,
    updated_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticResolutionResponse {
    resolutions: Vec<DiagnosticResolution>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticBatch<'a> {
    instance_id: &'a str,
    reports: &'a [DiagnosticReportPreview],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticBatchAck {
    accepted_report_ids: Vec<String>,
}

struct RunningGuard<'a>(&'a AtomicBool);

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl DiagnosticState {
    pub fn open(path: &Path, panic_marker_path: &Path) -> Result<Self, String> {
        let configured_api_base = std::env::var("LYNX_DIAGNOSTIC_API_BASE")
            .unwrap_or_else(|_| DEFAULT_API_BASE.to_owned());
        let api_base = if validate_api_base(&configured_api_base).is_ok() {
            configured_api_base
        } else {
            log::warn!("LYNX diagnostic API override was rejected; using the secure default");
            DEFAULT_API_BASE.to_owned()
        };
        match Self::open_with_base(path, &api_base, panic_marker_path) {
            Ok(state) => Ok(state),
            Err(_) => {
                log::warn!("LYNX diagnostics local storage degraded; using bounded memory state");
                let state =
                    Self::open_with_base(Path::new(":memory:"), &api_base, panic_marker_path)?;
                record_meta_error(&state, "DIAGNOSTIC_LOCAL_STORAGE_DEGRADED")?;
                Ok(state)
            }
        }
    }

    fn open_with_base(
        path: &Path,
        api_base: &str,
        panic_marker_path: &Path,
    ) -> Result<Self, String> {
        validate_api_base(api_base)?;
        if let Some(parent) = path.parent().filter(|item| !item.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let db = Connection::open(path).map_err(|error| error.to_string())?;
        migrate(&db).map_err(|error| error.to_string())?;
        let state = Self {
            db: Mutex::new(db),
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|error| error.to_string())?,
            api_base: api_base.trim_end_matches('/').to_owned(),
            running: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            logout_epoch: AtomicU64::new(0),
            logout_cleanup_pending: AtomicBool::new(false),
        };
        if state.import_panic_marker(panic_marker_path).is_err() {
            log::warn!("LYNX diagnostic panic marker was invalid and has been discarded");
            let _ = std::fs::remove_file(panic_marker_path);
            record_meta_error(&state, "DIAGNOSTIC_PANIC_MARKER_INVALID")?;
        }
        if state.import_native_operation_marker().is_err() {
            if let Some(runtime) = NATIVE_OPERATION_MARKER.get() {
                if let Ok(runtime) = runtime.lock() {
                    let _ = std::fs::remove_file(&runtime.path);
                }
            }
            record_meta_error(&state, "DIAGNOSTIC_NATIVE_MARKER_INVALID")?;
        }
        Ok(state)
    }

    fn config(&self) -> Result<DiagnosticConfig, String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        load_config(&db).map_err(|error| error.to_string())
    }

    fn set_enabled(&self, enabled: bool, user_disabled: bool) -> Result<(), String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        let consent_app_version = enabled.then_some(env!("CARGO_PKG_VERSION"));
        db.execute(
            "UPDATE diagnostic_config
             SET enabled=?1,consent_app_version=?2,user_disabled=?3,updated_at=?4 WHERE id=1",
            params![enabled, consent_app_version, user_disabled, now_seconds()],
        )
        .map_err(|error| error.to_string())?;
        db.execute(
            "UPDATE diagnostic_meta SET last_error_code=NULL WHERE id=1",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn set_enabled_runtime(&self, enabled: bool) -> Result<(), String> {
        if !enabled {
            self.suspended.store(true, Ordering::Release);
        }
        if enabled && self.logout_cleanup_pending.load(Ordering::Acquire) {
            let logout_epoch = self.logout_epoch.load(Ordering::Acquire);
            self.reset_for_logout(logout_epoch)?;
        }
        self.set_enabled(enabled, !enabled)?;
        self.suspended.store(!enabled, Ordering::Release);
        Ok(())
    }

    fn suspend_for_invalid_auth(&self) -> Result<(), String> {
        self.suspended.store(true, Ordering::Release);
        let user_disabled = self.config()?.user_disabled;
        self.set_enabled(false, user_disabled)
    }

    fn reset_for_logout(&self, logout_epoch: u64) -> Result<(), String> {
        let mut db = self.db.lock().map_err(|error| error.to_string())?;
        if !self.suspended.load(Ordering::Acquire)
            || self.logout_epoch.load(Ordering::Acquire) != logout_epoch
        {
            return Ok(());
        }
        let transaction = db.transaction().map_err(|error| error.to_string())?;
        let instance_id = format!("lynx-diagnostic-{:032x}", rand::random::<u128>());
        transaction
            .execute(
                "UPDATE diagnostic_config
                 SET enabled=0,instance_id=?1,consent_app_version=NULL,user_disabled=0,updated_at=?2 WHERE id=1",
                params![instance_id, now_seconds()],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM diagnostic_reports", [])
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM diagnostic_incidents", [])
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE diagnostic_meta
                 SET last_attempt_at=NULL,last_success_at=NULL,last_error_code=NULL WHERE id=1",
                [],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        self.logout_cleanup_pending.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn schedule_logout_reset(self: Arc<Self>) {
        self.suspended.store(true, Ordering::Release);
        self.logout_cleanup_pending.store(true, Ordering::Release);
        let logout_epoch = self.logout_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        tauri::async_runtime::spawn_blocking(move || {
            if self.reset_for_logout(logout_epoch).is_err() {
                log::warn!(
                    "LYNX diagnostic logout cleanup was deferred after a local storage error"
                );
            }
        });
    }

    fn status(&self) -> Result<DiagnosticReportingStatus, String> {
        let db = self.db.lock().map_err(|error| error.to_string())?;
        let mut status = status_from_db(&db).map_err(|error| error.to_string())?;
        if self.suspended.load(Ordering::Acquire) {
            status.enabled = false;
            status.state = "disabled".to_owned();
        }
        if status.enabled && self.running.load(Ordering::Acquire) {
            status.state = "uploading".to_owned();
        }
        Ok(status)
    }

    fn capture(
        &self,
        request: DiagnosticCaptureRequest,
    ) -> Result<DiagnosticCaptureResult, String> {
        let report = normalize_report(request)?;
        persist_report(self, &report)
    }

    fn import_panic_marker(&self, marker_path: &Path) -> Result<(), String> {
        let marker = match std::fs::read_to_string(marker_path) {
            Ok(marker) => marker,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct PanicMarker {
            occurred_at: i64,
            location_fingerprint: String,
        }
        let marker: PanicMarker = serde_json::from_str(&marker)
            .map_err(|_| "DIAGNOSTIC_PANIC_MARKER_INVALID".to_owned())?;
        let request = DiagnosticCaptureRequest {
            source: "rust_panic".to_owned(),
            component: "desktop_backend".to_owned(),
            severity: "fatal".to_owned(),
            error_code: "RUST_PANIC".to_owned(),
            machine_code: Some("RUST_PANIC".to_owned()),
            message_template: Some("Rust backend panic".to_owned()),
            stack: None,
            operation_type: Some("application_lifecycle".to_owned()),
            task_phase: Some("runtime".to_owned()),
            correlation_id: None,
            failure_class: None,
            resolution_key: None,
            retryable: None,
            recommended_skill: None,
        };
        let mut report = normalize_report(request)?;
        report.occurred_at = marker.occurred_at;
        report.last_occurred_at = marker.occurred_at;
        report.stack_fingerprint = Some(
            if marker.location_fingerprint.len() == 64
                && marker
                    .location_fingerprint
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit())
            {
                marker.location_fingerprint.to_ascii_lowercase()
            } else {
                sha256_hex(marker.location_fingerprint.as_bytes())
            },
        );
        report.fingerprint = report_fingerprint(&report);
        persist_report(self, &report)?;
        std::fs::remove_file(marker_path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn import_native_operation_marker(&self) -> Result<(), String> {
        let Some(runtime) = NATIVE_OPERATION_MARKER.get() else {
            return Ok(());
        };
        let path = runtime
            .lock()
            .map_err(|_| "DIAGNOSTIC_NATIVE_MARKER_LOCK".to_owned())?
            .path
            .clone();
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let entries: Vec<NativeOperationEntry> = serde_json::from_str(&content)
            .map_err(|_| "DIAGNOSTIC_NATIVE_MARKER_INVALID".to_owned())?;
        let entry = entries
            .into_iter()
            .min_by_key(|entry| entry.started_at)
            .ok_or_else(|| "DIAGNOSTIC_NATIVE_MARKER_INVALID".to_owned())?;
        if !valid_native_operation_type(&entry.operation_type) {
            return Err("DIAGNOSTIC_NATIVE_MARKER_INVALID".to_owned());
        }
        let mut report = normalize_report(DiagnosticCaptureRequest {
            source: "native_runtime".to_owned(),
            component: "ssh_transport".to_owned(),
            severity: "fatal".to_owned(),
            error_code: "NATIVE_OPERATION_INTERRUPTED".to_owned(),
            machine_code: Some("NATIVE_OPERATION_INTERRUPTED".to_owned()),
            message_template: Some("Native SSH operation ended with the process".to_owned()),
            stack: None,
            operation_type: Some(entry.operation_type),
            task_phase: Some("native_transport".to_owned()),
            correlation_id: None,
            failure_class: None,
            resolution_key: None,
            retryable: None,
            recommended_skill: None,
        })?;
        report.occurred_at = entry.started_at;
        report.last_occurred_at = entry.started_at;
        report.fingerprint = report_fingerprint(&report);
        persist_report(self, &report)?;
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub fn configure_panic_marker(path: PathBuf) {
    let _ = PANIC_MARKER_PATH.set(path);
}

pub fn configure_native_operation_marker(path: PathBuf) {
    let _ = NATIVE_OPERATION_MARKER.set(Mutex::new(NativeOperationMarkerRuntime {
        path,
        active: Vec::new(),
    }));
}

pub(crate) fn begin_native_ssh_operation(operation_type: &str) -> NativeOperationGuard {
    let operation_type = if valid_native_operation_type(operation_type) {
        operation_type.to_owned()
    } else {
        "ssh_native_operation".to_owned()
    };
    if let Some(runtime) = NATIVE_OPERATION_MARKER.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.active.push(NativeOperationEntry {
                operation_type: operation_type.clone(),
                started_at: now_seconds(),
            });
            persist_native_operation_marker(&runtime);
        }
    }
    NativeOperationGuard { operation_type }
}

fn valid_native_operation_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn persist_native_operation_marker(runtime: &NativeOperationMarkerRuntime) {
    if runtime.path.as_os_str().is_empty() {
        return;
    }
    if runtime.active.is_empty() {
        let _ = std::fs::remove_file(&runtime.path);
        return;
    }
    if let Ok(bytes) = serde_json::to_vec(&runtime.active) {
        let temporary = runtime.path.with_extension("json.new");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, &runtime.path);
        }
    }
}

pub fn record_panic_marker(location: Option<&std::panic::Location<'_>>) -> String {
    let location_hint = location
        .map(|item| {
            let file = Path::new(item.file())
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            format!("{file}:{}", item.line())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let fingerprint = sha256_hex(location_hint.as_bytes());
    if let Some(path) = PANIC_MARKER_PATH.get() {
        let marker = serde_json::json!({
            "occurredAt": now_seconds(),
            "locationFingerprint": fingerprint,
        });
        if let Ok(bytes) = serde_json::to_vec(&marker) {
            let _ = std::fs::write(path, bytes);
        }
    }
    fingerprint
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(REPORT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let state = app.state::<Arc<DiagnosticState>>().inner().clone();
            let auth = app.state::<Arc<AuthState>>().inner().clone();
            if let Err(code) = sync_once(&state, &auth, &app, false).await {
                log::debug!("LYNX diagnostics deferred: {code}");
            }
        }
    });
}

/// Enable diagnostics after Rust has validated an authenticated session. An
/// explicit settings opt-out is preserved across restarts and upgrades.
pub(crate) fn activate_for_authenticated_session(
    state: Arc<DiagnosticState>,
    auth: Arc<AuthState>,
    app: AppHandle,
) -> Result<(), String> {
    if state.config()?.user_disabled {
        return Ok(());
    }
    state.set_enabled(true, false)?;
    state.suspended.store(false, Ordering::Release);
    tauri::async_runtime::spawn(async move {
        if let Err(code) = sync_once(&state, &auth, &app, true).await {
            log::debug!("LYNX diagnostics login sync deferred: {code}");
        }
    });
    Ok(())
}

/// Capture a fixed, detail-free native background failure in the same bounded
/// queue used by frontend and IPC errors.
pub(crate) fn capture_backend_error(
    state: &DiagnosticState,
    component: &str,
    error_code: &str,
    operation_type: &str,
) {
    if state
        .capture(DiagnosticCaptureRequest {
            source: "backend".to_owned(),
            component: component.to_owned(),
            severity: "error".to_owned(),
            error_code: error_code.to_owned(),
            machine_code: Some(error_code.to_owned()),
            message_template: None,
            stack: None,
            operation_type: Some(operation_type.to_owned()),
            task_phase: Some("background".to_owned()),
            correlation_id: None,
            failure_class: None,
            resolution_key: None,
            retryable: None,
            recommended_skill: None,
        })
        .is_err()
    {
        log::warn!("LYNX background diagnostic event could not be stored locally");
    }
}

fn expected_control_flow_error(code: &str) -> bool {
    matches!(
        code,
        "HARDWARE_TASK_ACTIVE"
            | "SERIAL_OVERVIEW_NOT_OPEN"
            | "SERIAL_OVERVIEW_SHELL_NOT_CONFIRMED"
            | "ADB_DEVICE_NOT_READY"
            | "PORT_BUSY"
            | "SERIAL_ENUM_TIMEOUT"
            | "CH347F_DEVICE_NOT_FOUND"
            | "CH347F_DEVICE_SELECTION_REQUIRED"
            | "USER_CANCELLED"
            | "TASK_CANCELLED"
            | "CANCELLED"
    )
}

pub(crate) fn capture_audit_failure(app: &AppHandle, event: &crate::audit::AuditEventDraft) {
    if event.result != "failed" && event.result != "error" {
        return;
    }
    let error_code = event
        .error_code
        .as_deref()
        .unwrap_or("TASK_EXECUTION_FAILED");
    if expected_control_flow_error(error_code) {
        return;
    }
    let Some(state) = app.try_state::<Arc<DiagnosticState>>() else {
        return;
    };
    let _ = state.capture(DiagnosticCaptureRequest {
        source: "backend".to_owned(),
        component: bounded_identifier(&event.category, 64, "task", false),
        severity: "error".to_owned(),
        error_code: error_code.to_owned(),
        machine_code: Some(error_code.to_owned()),
        message_template: None,
        stack: None,
        operation_type: Some(event.action.clone()),
        task_phase: Some("terminal".to_owned()),
        correlation_id: event.correlation_id.clone(),
        failure_class: None,
        resolution_key: None,
        retryable: None,
        recommended_skill: None,
    });
}

fn rockchip_open_error_message(error_code: &str) -> Option<&'static str> {
    match error_code {
        "ROCKCHIP_USB_ACCESS_DENIED" => {
            Some("Rockchip USB access was denied; check permissions and the active driver")
        }
        "ROCKCHIP_USB_DEVICE_BUSY" => {
            Some("Rockchip USB device is busy; close other flashing tools and reconnect the device")
        }
        "ROCKCHIP_USB_DRIVER_UNSUPPORTED" => Some(
            "Rockchip USB driver is not libusb-compatible; bind a compatible driver and reconnect",
        ),
        "ROCKCHIP_USB_DEVICE_DISCONNECTED" => {
            Some("Rockchip USB device disconnected; reconnect it and enter Maskrom again")
        }
        "ROCKCHIP_USB_OPEN_FAILED" => {
            Some("Rockchip USB device could not be opened; check driver and exclusive access")
        }
        _ => None,
    }
}

/// Queue only the explicitly supported Rockchip Query Device failures. The
/// caller supplies no raw tool output, USB binding or firmware path.
pub(crate) fn capture_rockchip_open_error(
    state: &DiagnosticState,
    error_code: &str,
    task_phase: &str,
    task_id: &str,
) {
    let Some(message_template) = rockchip_open_error_message(error_code) else {
        return;
    };
    if state
        .capture(DiagnosticCaptureRequest {
            source: "backend".to_owned(),
            component: "workbench_flasher".to_owned(),
            severity: "warning".to_owned(),
            error_code: error_code.to_owned(),
            machine_code: Some(error_code.to_owned()),
            message_template: Some(message_template.to_owned()),
            stack: None,
            operation_type: Some("openrockcli_flash".to_owned()),
            task_phase: Some(task_phase.to_owned()),
            correlation_id: Some(task_id.to_owned()),
            failure_class: None,
            resolution_key: None,
            retryable: None,
            recommended_skill: None,
        })
        .is_err()
    {
        log::warn!("LYNX Rockchip open failure could not be stored locally");
    }
}

/// Attempt a prompt upload once the failed hardware task has had time to
/// release its guard. The normal periodic uploader remains the durable retry.
pub(crate) fn schedule_rockchip_open_error_sync(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let state = app.state::<Arc<DiagnosticState>>().inner().clone();
        let auth = app.state::<Arc<AuthState>>().inner().clone();
        if let Err(code) = sync_once(&state, &auth, &app, false).await {
            log::debug!("LYNX Rockchip open failure sync deferred: {code}");
        }
    });
}

#[tauri::command]
pub fn diagnostics_get_status(
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<DiagnosticReportingStatus, String> {
    state.status()
}

#[tauri::command]
pub async fn diagnostics_set_enabled(
    enabled: bool,
    state: State<'_, Arc<DiagnosticState>>,
    auth: State<'_, Arc<AuthState>>,
) -> Result<DiagnosticReportingStatus, String> {
    if enabled {
        auth.get_access_token().await.map_err(|error| {
            if error.starts_with("LINK_AUTH_REQUIRED") {
                error
            } else {
                "AUTH_TOKEN_EXPIRED:登录已过期，请重新登录".to_owned()
            }
        })?;
    }
    state.set_enabled_runtime(enabled)?;
    state.status()
}

#[tauri::command]
pub fn diagnostics_capture(
    request: DiagnosticCaptureRequest,
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<DiagnosticCaptureResult, String> {
    state.capture(request).map_err(|_| {
        log::warn!("LYNX diagnostic event could not be stored locally");
        "DIAGNOSTIC_LOCAL_STORE_FAILED".to_owned()
    })
}

#[tauri::command]
pub fn diagnostics_list_pending(
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<Vec<DiagnosticReportPreview>, String> {
    load_pending(state.inner(), MAX_PENDING_REPORTS, false)
}

#[tauri::command]
pub fn diagnostics_clear_pending(
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<DiagnosticReportingStatus, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute("DELETE FROM diagnostic_reports", [])
        .map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE diagnostic_meta SET last_error_code=NULL WHERE id=1",
        [],
    )
    .map_err(|error| error.to_string())?;
    drop(db);
    state.status()
}

#[tauri::command]
pub fn diagnostics_list_incidents(
    include_dismissed: Option<bool>,
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<Vec<DiagnosticIncident>, String> {
    load_incidents(
        state.inner(),
        include_dismissed.unwrap_or(false),
        MAX_LOCAL_INCIDENTS,
    )
}

#[tauri::command]
pub fn diagnostics_dismiss_incident(
    fingerprint: String,
    dismissed: bool,
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<(), String> {
    if !valid_fingerprint(&fingerprint) {
        return Err("DIAGNOSTIC_FINGERPRINT_INVALID".to_owned());
    }
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let changed = db
        .execute(
            "UPDATE diagnostic_incidents SET dismissed=?1 WHERE fingerprint=?2",
            params![dismissed, fingerprint],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("DIAGNOSTIC_INCIDENT_NOT_FOUND".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub fn diagnostics_build_ai_handoff(
    fingerprint: String,
    state: State<'_, Arc<DiagnosticState>>,
) -> Result<String, String> {
    if !valid_fingerprint(&fingerprint) {
        return Err("DIAGNOSTIC_FINGERPRINT_INVALID".to_owned());
    }
    let incident = load_incident(state.inner(), &fingerprint)?
        .ok_or_else(|| "DIAGNOSTIC_INCIDENT_NOT_FOUND".to_owned())?;
    Ok(format!(
        "请诊断 LYNX 脱敏故障。errorCode={}，component={}，failureClass={}，resolutionKey={}，retryable={}，taskPhase={}，recommendedSkill={}。先通过当前 LYNX MCP 读取指定 Skill，再提出只读诊断步骤；不要自动重试硬件动作，不要改变板卡作用域。",
        incident.error_code,
        incident.component,
        incident.failure_class,
        incident.resolution_key,
        incident.retryable,
        incident.task_phase.as_deref().unwrap_or("unknown"),
        incident.recommended_skill.as_deref().unwrap_or("none"),
    ))
}

#[tauri::command]
pub async fn diagnostics_refresh_resolutions(
    state: State<'_, Arc<DiagnosticState>>,
    auth: State<'_, Arc<AuthState>>,
) -> Result<Vec<DiagnosticIncident>, String> {
    let incidents = load_incidents(state.inner(), false, MAX_RESOLUTION_QUERY)?;
    if incidents.is_empty() {
        return Ok(incidents);
    }
    let fingerprints: Vec<String> = incidents
        .iter()
        .map(|incident| incident.fingerprint.clone())
        .collect();
    let token = auth
        .get_access_token()
        .await
        .map_err(|_| "AUTH_REQUIRED".to_owned())?;
    query_resolutions(state.inner(), &token, &fingerprints).await?;
    load_incidents(state.inner(), false, MAX_LOCAL_INCIDENTS)
}

#[tauri::command]
pub async fn diagnostics_sync_now(
    app: AppHandle,
    state: State<'_, Arc<DiagnosticState>>,
    auth: State<'_, Arc<AuthState>>,
) -> Result<DiagnosticReportingStatus, String> {
    sync_once(state.inner(), auth.inner(), &app, true).await?;
    state.status()
}

async fn sync_once(
    state: &Arc<DiagnosticState>,
    auth: &Arc<AuthState>,
    app: &AppHandle,
    force: bool,
) -> Result<(), String> {
    if state.suspended.load(Ordering::Acquire) {
        return Err("DIAGNOSTIC_REPORTING_DISABLED".to_owned());
    }
    let config = state.config()?;
    if !config.enabled {
        return Err("DIAGNOSTIC_REPORTING_DISABLED".to_owned());
    }
    if state.running.swap(true, Ordering::AcqRel) {
        return Err("DIAGNOSTIC_UPLOAD_ALREADY_RUNNING".to_owned());
    }
    let _guard = RunningGuard(&state.running);
    if hardware_safety::activity_may_be_active_nonblocking(app) {
        return Err("DIAGNOSTIC_DEFERRED_HARDWARE_ACTIVE".to_owned());
    }

    cleanup_expired(state)?;
    let reports = load_pending(state, MAX_BATCH_REPORTS, !force)?;
    if reports.is_empty() {
        return Ok(());
    }
    let token = match auth.get_access_token().await {
        Ok(token) => token,
        Err(_) => {
            state.suspend_for_invalid_auth()?;
            record_meta_error(state, "AUTH_REQUIRED")?;
            return Err("AUTH_REQUIRED".to_owned());
        }
    };
    if state.suspended.load(Ordering::Acquire) {
        return Err("DIAGNOSTIC_REPORTING_DISABLED".to_owned());
    }
    record_attempt(state)?;
    let accepted = post_batch(state, &token, &config.instance_id, &reports).await?;
    if accepted.is_empty() {
        record_rejected(state, "DIAGNOSTIC_INVALID_ACK", &reports)?;
        return Err("DIAGNOSTIC_INVALID_ACK".to_owned());
    }
    acknowledge_reports(state, &accepted)?;
    let accepted: std::collections::HashSet<&str> = accepted.iter().map(String::as_str).collect();
    let remaining: Vec<_> = reports
        .iter()
        .filter(|report| !accepted.contains(report.report_id.as_str()))
        .cloned()
        .collect();
    if !remaining.is_empty() {
        record_retryable_failure(state, "DIAGNOSTIC_PARTIAL_ACK", &remaining)?;
        return Err("DIAGNOSTIC_PARTIAL_ACK".to_owned());
    }
    Ok(())
}

async fn post_batch(
    state: &DiagnosticState,
    token: &str,
    instance_id: &str,
    reports: &[DiagnosticReportPreview],
) -> Result<Vec<String>, String> {
    // Older queued rows may have been captured by a client build whose local
    // fallback values were broader than the public server contract. Normalize
    // a wire-only copy so those rows can be accepted without mutating or
    // discarding the user's local queue before an explicit ACK arrives.
    let upload_reports: Vec<_> = reports.iter().map(prepare_report_for_upload).collect();
    let response = state
        .client
        .post(format!("{}/reports/batch", state.api_base))
        .bearer_auth(token)
        .json(&DiagnosticBatch {
            instance_id,
            reports: &upload_reports,
        })
        .send()
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE".to_owned());
    let response = match response {
        Ok(response) => response,
        Err(code) => {
            record_retryable_failure(state, &code, reports)?;
            return Err(code);
        }
    };
    if response.status().is_success() {
        let ack: DiagnosticBatchAck = match response.json().await {
            Ok(ack) => ack,
            Err(_) => {
                record_rejected(state, "DIAGNOSTIC_INVALID_ACK", reports)?;
                return Err("DIAGNOSTIC_INVALID_ACK".to_owned());
            }
        };
        let requested: std::collections::HashSet<&str> =
            reports.iter().map(|item| item.report_id.as_str()).collect();
        if ack
            .accepted_report_ids
            .iter()
            .any(|item| !requested.contains(item.as_str()))
        {
            record_rejected(state, "DIAGNOSTIC_INVALID_ACK", reports)?;
            return Err("DIAGNOSTIC_INVALID_ACK".to_owned());
        }
        return Ok(ack.accepted_report_ids);
    }
    let status = response.status();
    log::warn!(
        "LYNX diagnostic upload rejected by server: HTTP {}",
        status.as_u16()
    );
    let code = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "AUTH_REQUIRED",
        StatusCode::TOO_MANY_REQUESTS => "RATE_LIMITED",
        status if status.is_server_error() => "DIAGNOSTIC_SERVER_ERROR",
        _ => "DIAGNOSTIC_REQUEST_REJECTED",
    };
    if matches!(
        code,
        "AUTH_REQUIRED" | "RATE_LIMITED" | "DIAGNOSTIC_SERVER_ERROR"
    ) {
        record_retryable_failure(state, code, reports)?;
    } else {
        record_rejected(state, code, reports)?;
    }
    Err(code.to_owned())
}

async fn query_resolutions(
    state: &DiagnosticState,
    token: &str,
    fingerprints: &[String],
) -> Result<(), String> {
    if fingerprints.is_empty() {
        return Ok(());
    }
    if fingerprints.len() > MAX_RESOLUTION_QUERY
        || fingerprints.iter().any(|item| !valid_fingerprint(item))
    {
        return Err("DIAGNOSTIC_RESOLUTION_QUERY_INVALID".to_owned());
    }
    let response = state
        .client
        .post(format!("{}/resolutions/query", state.api_base))
        .bearer_auth(token)
        .json(&DiagnosticResolutionQuery { fingerprints })
        .send()
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE".to_owned())?;
    if !response.status().is_success() {
        return Err(match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "AUTH_REQUIRED",
            StatusCode::TOO_MANY_REQUESTS => "RATE_LIMITED",
            status if status.is_server_error() => "DIAGNOSTIC_SERVER_ERROR",
            _ => "DIAGNOSTIC_RESOLUTION_QUERY_REJECTED",
        }
        .to_owned());
    }
    let payload: DiagnosticResolutionResponse = response
        .json()
        .await
        .map_err(|_| "DIAGNOSTIC_RESOLUTION_RESPONSE_INVALID".to_owned())?;
    let requested: std::collections::HashSet<&str> =
        fingerprints.iter().map(String::as_str).collect();
    let db = state.db.lock().map_err(|error| error.to_string())?;
    for resolution in payload.resolutions {
        if !requested.contains(resolution.fingerprint.as_str())
            || !valid_fingerprint(&resolution.fingerprint)
        {
            return Err("DIAGNOSTIC_RESOLUTION_RESPONSE_INVALID".to_owned());
        }
        let Some(resolution_key) = allowed_resolution_key(&resolution.resolution_key) else {
            return Err("DIAGNOSTIC_RESOLUTION_RESPONSE_INVALID".to_owned());
        };
        let recommended_skill = resolution
            .recommended_skill
            .as_deref()
            .and_then(allowed_recommended_skill);
        let public_workaround = resolution
            .public_workaround
            .as_deref()
            .and_then(sanitize_message);
        let status = bounded_identifier(&resolution.status, 32, "open", false);
        let target_version = resolution
            .target_version
            .as_deref()
            .filter(|value| value.len() <= 32)
            .map(str::to_owned);
        let released_version = resolution
            .released_version
            .as_deref()
            .filter(|value| value.len() <= 32)
            .map(str::to_owned);
        db.execute(
            "UPDATE diagnostic_incidents SET resolution_status=?1,resolution_key=?2,
             public_workaround=?3,target_version=?4,released_version=?5,recommended_skill=?6,
             resolution_updated_at=?7 WHERE fingerprint=?8",
            params![
                status,
                resolution_key,
                public_workaround,
                target_version,
                released_version,
                recommended_skill,
                resolution.updated_at,
                resolution.fingerprint,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS diagnostic_config(
            id INTEGER PRIMARY KEY CHECK(id=1),
            enabled INTEGER NOT NULL DEFAULT 0,
            instance_id TEXT NOT NULL,
            consent_app_version TEXT,
            user_disabled INTEGER NOT NULL DEFAULT 0,
            wire_revision INTEGER NOT NULL DEFAULT 2,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS diagnostic_reports(
            report_id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            occurred_at INTEGER NOT NULL,
            last_occurred_at INTEGER NOT NULL,
            occurrence_count INTEGER NOT NULL,
            app_version TEXT NOT NULL,
            build_commit TEXT NOT NULL,
            platform TEXT NOT NULL,
            architecture TEXT NOT NULL,
            source TEXT NOT NULL,
            component TEXT NOT NULL,
            severity TEXT NOT NULL,
            error_code TEXT NOT NULL,
            machine_code TEXT,
            message_template TEXT,
            stack_fingerprint TEXT,
            fingerprint TEXT NOT NULL,
            operation_type TEXT,
            task_phase TEXT,
            correlation_id TEXT,
            failure_class TEXT NOT NULL DEFAULT 'unknown',
            resolution_key TEXT NOT NULL DEFAULT 'contact_support',
            retryable INTEGER NOT NULL DEFAULT 0,
            recommended_skill TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt_at INTEGER NOT NULL DEFAULT 0,
            UNIQUE(fingerprint,app_version)
         );
         CREATE TABLE IF NOT EXISTS diagnostic_meta(
            id INTEGER PRIMARY KEY CHECK(id=1),
            last_attempt_at INTEGER,
            last_success_at INTEGER,
            last_error_code TEXT
         );
         CREATE TABLE IF NOT EXISTS diagnostic_incidents(
            fingerprint TEXT PRIMARY KEY,
            first_occurred_at INTEGER NOT NULL,
            last_occurred_at INTEGER NOT NULL,
            occurrence_count INTEGER NOT NULL,
            component TEXT NOT NULL,
            severity TEXT NOT NULL,
            error_code TEXT NOT NULL,
            machine_code TEXT,
            operation_type TEXT,
            task_phase TEXT,
            failure_class TEXT NOT NULL,
            resolution_key TEXT NOT NULL,
            retryable INTEGER NOT NULL,
            recommended_skill TEXT,
            upload_state TEXT NOT NULL DEFAULT 'pending',
            dismissed INTEGER NOT NULL DEFAULT 0,
            resolution_status TEXT,
            public_workaround TEXT,
            target_version TEXT,
            released_version TEXT,
            resolution_updated_at INTEGER
         );",
    )?;
    if !column_exists(db, "diagnostic_config", "consent_app_version")? {
        db.execute(
            "ALTER TABLE diagnostic_config ADD COLUMN consent_app_version TEXT",
            [],
        )?;
    }
    if !column_exists(db, "diagnostic_config", "wire_revision")? {
        db.execute(
            "ALTER TABLE diagnostic_config ADD COLUMN wire_revision INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(db, "diagnostic_config", "user_disabled")? {
        db.execute(
            "ALTER TABLE diagnostic_config ADD COLUMN user_disabled INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    for (column, declaration) in [
        ("failure_class", "TEXT NOT NULL DEFAULT 'unknown'"),
        ("resolution_key", "TEXT NOT NULL DEFAULT 'contact_support'"),
        ("retryable", "INTEGER NOT NULL DEFAULT 0"),
        ("recommended_skill", "TEXT"),
    ] {
        if !column_exists(db, "diagnostic_reports", column)? {
            db.execute(
                &format!("ALTER TABLE diagnostic_reports ADD COLUMN {column} {declaration}"),
                [],
            )?;
        }
    }
    let instance_id = format!("lynx-diagnostic-{:032x}", rand::random::<u128>());
    db.execute(
        "INSERT OR IGNORE INTO diagnostic_config(
            id,enabled,instance_id,consent_app_version,user_disabled,wire_revision,updated_at)
         VALUES(1,0,?1,NULL,0,?2,?3)",
        params![instance_id, DIAGNOSTIC_WIRE_REVISION, now_seconds()],
    )?;
    let wire_revision: i64 = db.query_row(
        "SELECT wire_revision FROM diagnostic_config WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    if wire_revision < DIAGNOSTIC_WIRE_REVISION {
        db.execute(
            "UPDATE diagnostic_reports SET attempts=0,next_attempt_at=0",
            [],
        )?;
        db.execute(
            "UPDATE diagnostic_config SET wire_revision=?1,updated_at=?2 WHERE id=1",
            params![DIAGNOSTIC_WIRE_REVISION, now_seconds()],
        )?;
    }
    db.execute("INSERT OR IGNORE INTO diagnostic_meta(id) VALUES(1)", [])?;
    Ok(())
}

fn column_exists(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for item in columns {
        if item? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_config(db: &Connection) -> rusqlite::Result<DiagnosticConfig> {
    db.query_row(
        "SELECT enabled,instance_id,user_disabled
         FROM diagnostic_config WHERE id=1",
        [],
        |row| {
            let enabled: bool = row.get(0)?;
            Ok(DiagnosticConfig {
                enabled,
                instance_id: row.get(1)?,
                user_disabled: row.get(2)?,
            })
        },
    )
}

fn classify_incident(
    error_code: &str,
    component: &str,
) -> (&'static str, &'static str, bool, Option<&'static str>) {
    let code = error_code.to_ascii_uppercase();
    let component = component.to_ascii_lowercase();
    if code.starts_with("ROCKCHIP_USB_") {
        let action = if code.contains("BUSY") || code.contains("OPEN_FAILED") {
            "close_conflicting_tool"
        } else if code.contains("DISCONNECTED") {
            "reconnect_device"
        } else {
            "install_compatible_driver"
        };
        return ("driver", action, true, Some("linux-incident-triage"));
    }
    if code.starts_with("SERIAL_") || code == "PORT_BUSY" {
        return (
            "driver",
            "reconnect_device",
            true,
            Some("serial-diagnostics"),
        );
    }
    if code.starts_with("ADB_") || code.starts_with("USB_") || code.starts_with("CH347") {
        return (
            "hardware",
            "reconnect_device",
            true,
            Some("linux-incident-triage"),
        );
    }
    if code.starts_with("AUTH_") || code.contains("UNAUTHORIZED") || code.contains("FORBIDDEN") {
        return ("permission", "reauthenticate", true, None);
    }
    if code.contains("NETWORK") || code.contains("TIMEOUT") || code.contains("DOWNLOAD") {
        return (
            "connectivity",
            "check_network",
            true,
            Some("linux-incident-triage"),
        );
    }
    if code.contains("CONFIG") || code.contains("INVALID") || code.contains("UNSUPPORTED") {
        return ("configuration", "check_configuration", false, None);
    }
    if code.contains("STORAGE") || code.contains("DATABASE") || code.contains("CORRUPT") {
        return ("data", "repair_local_data", false, None);
    }
    if code.contains("BUSY") || code.contains("RESOURCE") || code.contains("MEMORY") {
        return ("resource", "retry", true, None);
    }
    let skill = if component.starts_with("mcu") {
        Some("mcu-incident-triage")
    } else {
        Some("linux-incident-triage")
    };
    ("software", "update_lynx", false, skill)
}

fn allowed_failure_class(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "software" => Some("software"),
        "hardware" => Some("hardware"),
        "driver" => Some("driver"),
        "permission" => Some("permission"),
        "configuration" => Some("configuration"),
        "connectivity" => Some("connectivity"),
        "resource" => Some("resource"),
        "data" => Some("data"),
        "unknown" => Some("unknown"),
        _ => None,
    }
}

fn allowed_resolution_key(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "retry" => Some("retry"),
        "reconnect_device" => Some("reconnect_device"),
        "close_conflicting_tool" => Some("close_conflicting_tool"),
        "install_compatible_driver" => Some("install_compatible_driver"),
        "reauthenticate" => Some("reauthenticate"),
        "check_configuration" => Some("check_configuration"),
        "check_network" => Some("check_network"),
        "repair_local_data" => Some("repair_local_data"),
        "update_lynx" => Some("update_lynx"),
        "contact_support" => Some("contact_support"),
        _ => None,
    }
}

fn allowed_recommended_skill(value: &str) -> Option<&'static str> {
    match value.trim() {
        "linux-incident-triage" => Some("linux-incident-triage"),
        "mcu-incident-triage" => Some("mcu-incident-triage"),
        "serial-diagnostics" => Some("serial-diagnostics"),
        "allwinner-fes-flashing" => Some("allwinner-fes-flashing"),
        "t113s3-mainline-nand" => Some("t113s3-mainline-nand"),
        "t527-sdk-emmc" => Some("t527-sdk-emmc"),
        "freertos-cherryusb" => Some("freertos-cherryusb"),
        _ => None,
    }
}

fn normalize_report(request: DiagnosticCaptureRequest) -> Result<DiagnosticReportPreview, String> {
    let source = match request.source.trim().to_ascii_lowercase().as_str() {
        "frontend" => "frontend",
        "backend" => "backend",
        "rust_panic" | "lifecycle" => "system",
        _ => "system",
    }
    .to_owned();
    let component = bounded_identifier(&request.component, 64, "unknown_component", false);
    let severity = match request.severity.trim().to_ascii_lowercase().as_str() {
        "warning" => "warning",
        "error" => "error",
        "fatal" => "fatal",
        _ => "error",
    }
    .to_owned();
    let error_code = sanitized_error_code(&request.error_code);
    let machine_code = request.machine_code.as_deref().map(sanitized_error_code);
    let message_template = request
        .message_template
        .as_deref()
        .and_then(sanitize_message);
    let stack_fingerprint = request
        .stack
        .as_deref()
        .and_then(|stack| sanitize_message(stack).map(|safe| sha256_hex(safe.as_bytes())));
    let operation_type = request
        .operation_type
        .as_deref()
        .map(|value| bounded_identifier(value, 64, "unknown_operation", false));
    let task_phase = request
        .task_phase
        .as_deref()
        .map(|value| bounded_identifier(value, 64, "unknown_phase", false));
    let correlation_id = request
        .correlation_id
        .as_deref()
        .map(|value| format!("corr-{}", &sha256_hex(value.as_bytes())[..32]));
    let classified = classify_incident(&error_code, &component);
    let failure_class = request
        .failure_class
        .as_deref()
        .and_then(allowed_failure_class)
        .unwrap_or(classified.0)
        .to_owned();
    let resolution_key = request
        .resolution_key
        .as_deref()
        .and_then(allowed_resolution_key)
        .unwrap_or(classified.1)
        .to_owned();
    let retryable = request.retryable.unwrap_or(classified.2);
    let recommended_skill = request
        .recommended_skill
        .as_deref()
        .and_then(allowed_recommended_skill)
        .or(classified.3)
        .map(str::to_owned);
    let now = now_seconds();
    let mut report = DiagnosticReportPreview {
        report_id: format!("diag-{:032x}", rand::random::<u128>()),
        schema_version: SCHEMA_VERSION,
        occurred_at: now,
        last_occurred_at: now,
        occurrence_count: 1,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        build_commit: normalize_build_commit(option_env!("LYNX_BUILD_COMMIT").unwrap_or("")),
        platform: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        source,
        component,
        severity,
        error_code,
        machine_code,
        message_template,
        stack_fingerprint,
        fingerprint: String::new(),
        operation_type,
        task_phase,
        correlation_id,
        failure_class,
        resolution_key,
        retryable,
        recommended_skill,
    };
    report.fingerprint = report_fingerprint(&report);
    let encoded = serde_json::to_vec(&report).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_REPORT_BYTES {
        return Err("DIAGNOSTIC_REPORT_TOO_LARGE".to_owned());
    }
    Ok(report)
}

fn prepare_report_for_upload(report: &DiagnosticReportPreview) -> DiagnosticReportPreview {
    let mut normalized = report.clone();
    normalized.build_commit = normalize_build_commit(&normalized.build_commit);
    normalized.source = match normalized.source.as_str() {
        "frontend" => "frontend",
        "backend" => "backend",
        _ => "system",
    }
    .to_owned();
    normalized.correlation_id = normalized
        .correlation_id
        .filter(|value| is_correlation_id(value));
    if normalized
        .message_template
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        // Older 100ask.net deployments treated this optional field as
        // required. Use the already-sanitized stable error code only on the
        // wire so queued reports remain compatible without adding user data.
        normalized.message_template = Some(normalized.error_code.clone());
    }
    normalized
}

fn normalize_build_commit(value: &str) -> String {
    let value = value.trim();
    if is_hex_commit(value) {
        value.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn is_hex_commit(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_correlation_id(value: &str) -> bool {
    value.len() == 37
        && value.starts_with("corr-")
        && value[5..]
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
}

fn report_fingerprint(report: &DiagnosticReportPreview) -> String {
    let material = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        report.source,
        report.component,
        report.severity,
        report.error_code,
        report.machine_code.as_deref().unwrap_or(""),
        report.message_template.as_deref().unwrap_or(""),
        report.stack_fingerprint.as_deref().unwrap_or(""),
        report.operation_type.as_deref().unwrap_or("")
    );
    sha256_hex(material.as_bytes())
}

fn persist_report(
    state: &DiagnosticState,
    report: &DiagnosticReportPreview,
) -> Result<DiagnosticCaptureResult, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO diagnostic_reports(
           report_id,schema_version,occurred_at,last_occurred_at,occurrence_count,
           app_version,build_commit,platform,architecture,source,component,severity,
           error_code,machine_code,message_template,stack_fingerprint,fingerprint,
           operation_type,task_phase,correlation_id,failure_class,resolution_key,retryable,
           recommended_skill,attempts,next_attempt_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,0,0)
         ON CONFLICT(fingerprint,app_version) DO UPDATE SET
           last_occurred_at=excluded.last_occurred_at,
           occurrence_count=diagnostic_reports.occurrence_count+1",
        params![
            report.report_id,
            report.schema_version,
            report.occurred_at,
            report.last_occurred_at,
            report.occurrence_count,
            report.app_version,
            report.build_commit,
            report.platform,
            report.architecture,
            report.source,
            report.component,
            report.severity,
            report.error_code,
            report.machine_code,
            report.message_template,
            report.stack_fingerprint,
            report.fingerprint,
            report.operation_type,
            report.task_phase,
            report.correlation_id,
            report.failure_class,
            report.resolution_key,
            report.retryable,
            report.recommended_skill,
        ],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO diagnostic_incidents(
           fingerprint,first_occurred_at,last_occurred_at,occurrence_count,component,severity,
           error_code,machine_code,operation_type,task_phase,failure_class,resolution_key,
           retryable,recommended_skill,upload_state,dismissed)
         VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'pending',0)
         ON CONFLICT(fingerprint) DO UPDATE SET
           last_occurred_at=excluded.last_occurred_at,
           occurrence_count=diagnostic_incidents.occurrence_count+1,
           component=excluded.component,severity=excluded.severity,error_code=excluded.error_code,
           machine_code=excluded.machine_code,operation_type=excluded.operation_type,
           task_phase=excluded.task_phase,failure_class=excluded.failure_class,
           resolution_key=excluded.resolution_key,retryable=excluded.retryable,
           recommended_skill=excluded.recommended_skill,upload_state='pending',dismissed=0",
        params![
            report.fingerprint,
            report.occurred_at,
            report.last_occurred_at,
            report.component,
            report.severity,
            report.error_code,
            report.machine_code,
            report.operation_type,
            report.task_phase,
            report.failure_class,
            report.resolution_key,
            report.retryable,
            report.recommended_skill,
        ],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM diagnostic_incidents WHERE fingerprint IN (
           SELECT fingerprint FROM diagnostic_incidents ORDER BY last_occurred_at DESC
           LIMIT -1 OFFSET ?1)",
        [MAX_LOCAL_INCIDENTS as i64],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM diagnostic_reports WHERE report_id IN (
           SELECT report_id FROM diagnostic_reports ORDER BY last_occurred_at DESC
           LIMIT -1 OFFSET ?1)",
        [MAX_PENDING_REPORTS as i64],
    )
    .map_err(|error| error.to_string())?;
    let stored_id: String = db
        .query_row(
            "SELECT report_id FROM diagnostic_reports WHERE fingerprint=?1 AND app_version=?2",
            params![report.fingerprint, report.app_version],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(DiagnosticCaptureResult {
        report_id: stored_id,
        fingerprint: report.fingerprint.clone(),
        queued: true,
    })
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn incident_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DiagnosticIncident> {
    Ok(DiagnosticIncident {
        fingerprint: row.get(0)?,
        first_occurred_at: row.get(1)?,
        last_occurred_at: row.get(2)?,
        occurrence_count: row.get(3)?,
        component: row.get(4)?,
        severity: row.get(5)?,
        error_code: row.get(6)?,
        machine_code: row.get(7)?,
        operation_type: row.get(8)?,
        task_phase: row.get(9)?,
        failure_class: row.get(10)?,
        resolution_key: row.get(11)?,
        retryable: row.get(12)?,
        recommended_skill: row.get(13)?,
        upload_state: row.get(14)?,
        dismissed: row.get(15)?,
        resolution_status: row.get(16)?,
        public_workaround: row.get(17)?,
        target_version: row.get(18)?,
        released_version: row.get(19)?,
        resolution_updated_at: row.get(20)?,
    })
}

fn incident_select() -> &'static str {
    "SELECT fingerprint,first_occurred_at,last_occurred_at,occurrence_count,component,severity,
     error_code,machine_code,operation_type,task_phase,failure_class,resolution_key,retryable,
     recommended_skill,upload_state,dismissed,resolution_status,public_workaround,target_version,
     released_version,resolution_updated_at FROM diagnostic_incidents"
}

fn load_incidents(
    state: &DiagnosticState,
    include_dismissed: bool,
    limit: usize,
) -> Result<Vec<DiagnosticIncident>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let sql = format!(
        "{} {} ORDER BY last_occurred_at DESC LIMIT ?1",
        incident_select(),
        if include_dismissed {
            ""
        } else {
            "WHERE dismissed=0"
        }
    );
    let mut statement = db.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([limit as i64], incident_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn load_incident(
    state: &DiagnosticState,
    fingerprint: &str,
) -> Result<Option<DiagnosticIncident>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let sql = format!("{} WHERE fingerprint=?1", incident_select());
    match db.query_row(&sql, [fingerprint], incident_from_row) {
        Ok(incident) => Ok(Some(incident)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn load_pending(
    state: &DiagnosticState,
    limit: usize,
    due_only: bool,
) -> Result<Vec<DiagnosticReportPreview>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let sql = if due_only {
        "SELECT report_id,schema_version,occurred_at,last_occurred_at,occurrence_count,
         app_version,build_commit,platform,architecture,source,component,severity,error_code,
         machine_code,message_template,stack_fingerprint,fingerprint,operation_type,task_phase,
         correlation_id,failure_class,resolution_key,retryable,recommended_skill
         FROM diagnostic_reports WHERE next_attempt_at<=?1
         ORDER BY occurred_at ASC LIMIT ?2"
    } else {
        "SELECT report_id,schema_version,occurred_at,last_occurred_at,occurrence_count,
         app_version,build_commit,platform,architecture,source,component,severity,error_code,
         machine_code,message_template,stack_fingerprint,fingerprint,operation_type,task_phase,
         correlation_id,failure_class,resolution_key,retryable,recommended_skill
         FROM diagnostic_reports
         ORDER BY occurred_at ASC LIMIT ?2"
    };
    let mut statement = db.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![now_seconds(), limit as i64], report_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn report_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DiagnosticReportPreview> {
    Ok(DiagnosticReportPreview {
        report_id: row.get(0)?,
        schema_version: row.get(1)?,
        occurred_at: row.get(2)?,
        last_occurred_at: row.get(3)?,
        occurrence_count: row.get(4)?,
        app_version: row.get(5)?,
        build_commit: row.get(6)?,
        platform: row.get(7)?,
        architecture: row.get(8)?,
        source: row.get(9)?,
        component: row.get(10)?,
        severity: row.get(11)?,
        error_code: row.get(12)?,
        machine_code: row.get(13)?,
        message_template: row.get(14)?,
        stack_fingerprint: row.get(15)?,
        fingerprint: row.get(16)?,
        operation_type: row.get(17)?,
        task_phase: row.get(18)?,
        correlation_id: row.get(19)?,
        failure_class: row.get(20)?,
        resolution_key: row.get(21)?,
        retryable: row.get(22)?,
        recommended_skill: row.get(23)?,
    })
}

fn cleanup_expired(state: &DiagnosticState) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM diagnostic_reports WHERE last_occurred_at<?1",
        [now_seconds().saturating_sub(RETENTION_SECONDS)],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM diagnostic_incidents WHERE last_occurred_at<?1",
        [now_seconds().saturating_sub(RETENTION_SECONDS)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn acknowledge_reports(state: &DiagnosticState, accepted: &[String]) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    for report_id in accepted {
        db.execute(
            "UPDATE diagnostic_incidents SET upload_state='uploaded'
             WHERE fingerprint=(SELECT fingerprint FROM diagnostic_reports WHERE report_id=?1)",
            [report_id],
        )
        .map_err(|error| error.to_string())?;
        db.execute(
            "DELETE FROM diagnostic_reports WHERE report_id=?1",
            [report_id],
        )
        .map_err(|error| error.to_string())?;
    }
    db.execute(
        "UPDATE diagnostic_meta SET last_attempt_at=?1,last_success_at=?1,last_error_code=NULL WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_attempt(state: &DiagnosticState) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE diagnostic_meta SET last_attempt_at=?1 WHERE id=1",
        [now_seconds()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_meta_error(state: &DiagnosticState, code: &str) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE diagnostic_meta SET last_attempt_at=?1,last_error_code=?2 WHERE id=1",
        params![
            now_seconds(),
            bounded_identifier(code, 64, "DIAGNOSTIC_FAILED", true)
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_retryable_failure(
    state: &DiagnosticState,
    code: &str,
    reports: &[DiagnosticReportPreview],
) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let attempts: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(attempts),0) FROM diagnostic_reports",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let attempts = attempts.saturating_add(1);
    let exponent = attempts.clamp(0, 6) as u32;
    let jitter = rand::random::<u8>() as i64 % 31;
    let backoff = (60_i64.saturating_mul(2_i64.pow(exponent)))
        .saturating_add(jitter)
        .min(MAX_BACKOFF_SECONDS);
    let next_attempt_at = now_seconds().saturating_add(backoff);
    for report in reports {
        db.execute(
            "UPDATE diagnostic_reports SET attempts=attempts+1,next_attempt_at=?1 WHERE report_id=?2",
            params![next_attempt_at, report.report_id],
        )
        .map_err(|error| error.to_string())?;
    }
    db.execute(
        "UPDATE diagnostic_meta SET last_attempt_at=?1,last_error_code=?2 WHERE id=1",
        params![
            now_seconds(),
            bounded_identifier(code, 64, "DIAGNOSTIC_FAILED", true)
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_rejected(
    state: &DiagnosticState,
    code: &str,
    reports: &[DiagnosticReportPreview],
) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    for report in reports {
        db.execute(
            "UPDATE diagnostic_reports SET next_attempt_at=?1 WHERE report_id=?2",
            params![i64::MAX / 2, report.report_id],
        )
        .map_err(|error| error.to_string())?;
    }
    db.execute(
        "UPDATE diagnostic_meta SET last_attempt_at=?1,last_error_code=?2 WHERE id=1",
        params![
            now_seconds(),
            bounded_identifier(code, 64, "DIAGNOSTIC_FAILED", true)
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn status_from_db(db: &Connection) -> rusqlite::Result<DiagnosticReportingStatus> {
    let config = load_config(db)?;
    let pending_count: usize =
        db.query_row("SELECT COUNT(*) FROM diagnostic_reports", [], |row| {
            row.get(0)
        })?;
    let (last_attempt_at, last_success_at, last_error_code): (
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = db.query_row(
        "SELECT last_attempt_at,last_success_at,last_error_code FROM diagnostic_meta WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let state = if !config.enabled {
        "disabled"
    } else if last_error_code.as_deref() == Some("AUTH_REQUIRED") {
        "auth_required"
    } else if matches!(
        last_error_code.as_deref(),
        Some("DIAGNOSTIC_REQUEST_REJECTED" | "DIAGNOSTIC_INVALID_ACK")
    ) {
        "rejected_schema"
    } else if pending_count > 0 && last_error_code.is_some() {
        "retry_wait"
    } else if pending_count > 0 {
        "pending"
    } else if last_success_at.is_some() {
        "synced"
    } else {
        "ready"
    };
    Ok(DiagnosticReportingStatus {
        enabled: config.enabled,
        state: state.to_owned(),
        pending_count,
        last_attempt_at,
        last_success_at,
        last_error_code,
    })
}

fn bounded_identifier(value: &str, max_len: usize, fallback: &str, uppercase: bool) -> String {
    let candidate = if uppercase {
        value.trim().to_ascii_uppercase()
    } else {
        value.trim().to_ascii_lowercase()
    };
    if candidate.is_empty()
        || candidate.len() > max_len
        || !candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        fallback.to_owned()
    } else {
        candidate
    }
}

fn sanitized_error_code(value: &str) -> String {
    let candidate = bounded_identifier(value, 64, "UNCLASSIFIED_ERROR", true);
    if candidate.len() >= 24 && !candidate.contains('_') {
        "UNCLASSIFIED_ERROR".to_owned()
    } else {
        candidate
    }
}

fn validate_api_base(value: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|_| "DIAGNOSTIC_API_URL_INVALID".to_owned())?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        return Err("DIAGNOSTIC_API_URL_INSECURE".to_owned());
    }
    if parsed.username() != "" || parsed.password().is_some() || parsed.query().is_some() {
        return Err("DIAGNOSTIC_API_URL_INVALID".to_owned());
    }
    Ok(())
}

fn sanitize_message(input: &str) -> Option<String> {
    let mut value: String = input
        .chars()
        .take(4096)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if value.trim().is_empty() {
        return None;
    }
    let rules = [
        (r"(?i)-----BEGIN [^-]{1,40}PRIVATE KEY-----", "[REDACTED]"),
        (r"(?i)\bBearer\s+[^\s,;]+", "Bearer [REDACTED]"),
        (
            r"(?i)\b(password|passwd|secret|token|api[_-]?key|authorization)\s*[:=]\s*[^\s,;]+",
            "$1=[REDACTED]",
        ),
        (
            r"(?i)\b(serial(?:number)?|device[_-]?id|location)\s*[:=]\s*[^\s,;]+",
            "$1=[REDACTED]",
        ),
        (r"(?i)(?:https?|file|tauri)://[^\s]+", "<URL>"),
        (r#"(?i)\b[A-Z]:[/\\][^\s"'<>]+"#, "<PATH>"),
        (
            r#"(?i)/(?:home|users|root|tmp|var|etc|mnt|opt|dev)/[^\s"'<>]+"#,
            "<PATH>",
        ),
        (
            r"(?i)\b(?:pciroot|usbroot|usb#vid_)[^\s,;]+",
            "<DEVICE_PATH>",
        ),
        (r"(?i)\blibusb:\d+(?::[\d.]+)+\b", "<DEVICE_PATH>"),
        (r"(?i)\bCOM\d+\b", "<DEVICE>"),
        (r"\b(?:\d{1,3}\.){3}\d{1,3}\b", "<IP>"),
        (r"(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b", "<MAC>"),
        (r"(?i)\b(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}\b", "<IP>"),
        (r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", "<EMAIL>"),
        (r#""[^"\r\n]{1,256}""#, "<VALUE>"),
        (r"'[^'\r\n]{1,256}'", "<VALUE>"),
        (r"`[^`\r\n]{1,256}`", "<VALUE>"),
        (r"\b[A-Za-z0-9+/_=-]{24,}\b", "[REDACTED]"),
    ];
    for (pattern, replacement) in rules {
        let regex = Regex::new(pattern).ok()?;
        value = regex.replace_all(&value, replacement).into_owned();
    }
    let whitespace = Regex::new(r"\s+").ok()?;
    value = whitespace.replace_all(value.trim(), " ").into_owned();
    let value: String = value.chars().take(512).collect();
    (!value.is_empty()).then_some(value)
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
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
    use axum::{http::HeaderMap, routing::post, Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn state() -> DiagnosticState {
        DiagnosticState::open_with_base(
            Path::new(":memory:"),
            "https://example.invalid/api/v1/lynx/diagnostics",
            Path::new("/path/that/does/not/exist"),
        )
        .expect("diagnostic state")
    }

    fn request(message: &str) -> DiagnosticCaptureRequest {
        DiagnosticCaptureRequest {
            source: "frontend".to_owned(),
            component: "webview".to_owned(),
            severity: "error".to_owned(),
            error_code: "WEBVIEW_UNHANDLED_ERROR".to_owned(),
            machine_code: None,
            message_template: Some(message.to_owned()),
            stack: Some("at render (/home/alice/project/App.tsx:12:4)".to_owned()),
            operation_type: Some("render".to_owned()),
            task_phase: None,
            correlation_id: Some("raw-user-session-id".to_owned()),
            failure_class: None,
            resolution_key: None,
            retryable: None,
            recommended_skill: None,
        }
    }

    #[test]
    fn rockchip_open_failures_are_actionable_and_detail_free() {
        let state = state();
        capture_rockchip_open_error(
            &state,
            "ROCKCHIP_USB_DEVICE_BUSY",
            "query_device",
            "flash-task-with-private-device-details",
        );
        capture_rockchip_open_error(&state, "UNRELATED_FLASH_FAILURE", "write", "ignored-task");

        let reports = load_pending(&state, 20, false).unwrap();
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.component, "workbench_flasher");
        assert_eq!(report.severity, "warning");
        assert_eq!(report.error_code, "ROCKCHIP_USB_DEVICE_BUSY");
        assert_eq!(
            report.machine_code.as_deref(),
            Some("ROCKCHIP_USB_DEVICE_BUSY")
        );
        assert_eq!(report.operation_type.as_deref(), Some("openrockcli_flash"));
        assert_eq!(report.task_phase.as_deref(), Some("query_device"));
        assert_eq!(report.failure_class, "driver");
        assert_eq!(report.resolution_key, "close_conflicting_tool");
        assert!(report.retryable);
        assert_eq!(
            report.recommended_skill.as_deref(),
            Some("linux-incident-triage")
        );
        assert!(report
            .message_template
            .as_deref()
            .is_some_and(|message| message.contains("close other flashing tools")));
        assert!(report
            .correlation_id
            .as_deref()
            .is_some_and(|value| value.starts_with("corr-")));
        let encoded = serde_json::to_string(report).unwrap();
        assert!(!encoded.contains("private-device-details"));
        assert!(!encoded.contains("ignored-task"));
        let incident = load_incident(&state, &report.fingerprint).unwrap().unwrap();
        assert_eq!(incident.occurrence_count, 1);
        assert_eq!(incident.upload_state, "pending");
        assert!(!incident.dismissed);
    }

    #[test]
    fn migration_defaults_disabled_and_instance_is_stable() {
        let state = state();
        let first = state.config().unwrap();
        let second = state.config().unwrap();
        assert!(!first.enabled);
        assert_eq!(first.instance_id, second.instance_id);
        assert!(first.instance_id.starts_with("lynx-diagnostic-"));
    }

    #[test]
    fn version_upgrade_preserves_enabled_authenticated_policy() {
        let state = state();
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "UPDATE diagnostic_config SET enabled=1,consent_app_version='0.0.0' WHERE id=1",
                [],
            )
            .unwrap();
        }
        assert!(state.config().unwrap().enabled);
        assert!(state.status().unwrap().enabled);
    }

    #[test]
    fn explicit_disable_is_distinct_from_auth_suspension() {
        let state = state();
        state.set_enabled_runtime(false).unwrap();
        assert!(state.config().unwrap().user_disabled);
        state.suspend_for_invalid_auth().unwrap();
        assert!(state.config().unwrap().user_disabled);
        state.set_enabled_runtime(true).unwrap();
        assert!(!state.config().unwrap().user_disabled);
    }

    #[test]
    fn wire_contract_upgrade_retries_previously_rejected_rows_once() {
        let state = state();
        state.capture(request("old wire contract")).unwrap();
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "UPDATE diagnostic_reports SET attempts=4,next_attempt_at=?1",
                [i64::MAX / 2],
            )
            .unwrap();
            db.execute(
                "UPDATE diagnostic_config SET wire_revision=2 WHERE id=1",
                [],
            )
            .unwrap();
            migrate(&db).unwrap();

            let revision: i64 = db
                .query_row(
                    "SELECT wire_revision FROM diagnostic_config WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let retry: (i64, i64) = db
                .query_row(
                    "SELECT attempts,next_attempt_at FROM diagnostic_reports",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(revision, DIAGNOSTIC_WIRE_REVISION);
            assert_eq!(retry, (0, 0));
        }

        // A second startup at the same revision must not keep resetting retry
        // state for a report the server has rejected under the current schema.
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "UPDATE diagnostic_reports SET attempts=1,next_attempt_at=?1",
                [i64::MAX / 2],
            )
            .unwrap();
            migrate(&db).unwrap();
            let retry: (i64, i64) = db
                .query_row(
                    "SELECT attempts,next_attempt_at FROM diagnostic_reports",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(retry, (1, i64::MAX / 2));
        }
    }

    #[test]
    fn logout_reset_clears_queue_disables_and_rotates_instance() {
        let state = state();
        state.set_enabled(true, false).unwrap();
        state.capture(request("queued before logout")).unwrap();
        let before = state.config().unwrap().instance_id;

        state.suspended.store(true, Ordering::Release);
        let epoch = state.logout_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        state.reset_for_logout(epoch).unwrap();

        let after = state.config().unwrap();
        assert!(!after.enabled);
        assert_ne!(before, after.instance_id);
        assert_eq!(state.status().unwrap().pending_count, 0);
        assert!(load_incidents(&state, true, MAX_LOCAL_INCIDENTS)
            .unwrap()
            .is_empty());
        assert_eq!(state.status().unwrap().state, "disabled");
    }

    #[test]
    fn next_account_cannot_enable_reporting_before_logout_queue_is_isolated() {
        let state = state();
        state.set_enabled(true, false).unwrap();
        state.capture(request("previous account report")).unwrap();
        state.suspended.store(true, Ordering::Release);
        state.logout_cleanup_pending.store(true, Ordering::Release);
        state.logout_epoch.fetch_add(1, Ordering::AcqRel);

        state.set_enabled_runtime(true).unwrap();

        let status = state.status().unwrap();
        assert!(status.enabled);
        assert_eq!(status.pending_count, 0);
        assert!(load_incidents(&state, true, MAX_LOCAL_INCIDENTS)
            .unwrap()
            .is_empty());
        assert!(!state.logout_cleanup_pending.load(Ordering::Acquire));
    }

    #[test]
    fn production_api_requires_https_and_no_embedded_credentials() {
        assert!(validate_api_base("https://100ask.net/api/v1/lynx/diagnostics").is_ok());
        assert!(validate_api_base("http://127.0.0.1:1234/test").is_ok());
        assert_eq!(
            validate_api_base("http://example.com/test").unwrap_err(),
            "DIAGNOSTIC_API_URL_INSECURE"
        );
        assert_eq!(
            validate_api_base("https://user:password@example.com/test").unwrap_err(),
            "DIAGNOSTIC_API_URL_INVALID"
        );
    }

    #[test]
    fn high_entropy_identifier_cannot_become_an_error_code() {
        assert_eq!(
            sanitized_error_code("ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"),
            "UNCLASSIFIED_ERROR"
        );
        assert_eq!(sanitized_error_code("RUST_PANIC"), "RUST_PANIC");
    }

    #[test]
    fn capture_source_is_restricted_to_the_public_schema() {
        let mut input = request("unexpected source");
        input.source = "serial_terminal_output".to_owned();
        let report = normalize_report(input).unwrap();
        assert_eq!(report.source, "system");
    }

    #[test]
    fn upload_payload_normalizes_existing_rows_to_the_server_contract() {
        let mut report = normalize_report(request("contract normalization")).unwrap();
        report.build_commit = "unknown".to_owned();
        report.source = "rust_panic".to_owned();
        report.correlation_id = Some("corr-too-short".to_owned());
        report.message_template = None;

        let normalized = prepare_report_for_upload(&report);

        assert_eq!(normalized.build_commit, "");
        assert_eq!(normalized.source, "system");
        assert_eq!(normalized.correlation_id, None);
        assert_eq!(
            normalized.message_template.as_deref(),
            Some(report.error_code.as_str())
        );
        assert_eq!(normalized.report_id, report.report_id);
        assert_eq!(normalized.fingerprint, report.fingerprint);
    }

    #[test]
    fn newly_captured_report_uses_server_compatible_identifiers() {
        let report = normalize_report(request("new report contract")).unwrap();

        assert!(report.build_commit.is_empty() || is_hex_commit(&report.build_commit));
        assert!(matches!(
            report.source.as_str(),
            "frontend" | "backend" | "system"
        ));
        assert_eq!(report.correlation_id.as_deref().map(str::len), Some(37));
    }

    #[test]
    fn capture_schema_rejects_unknown_fields() {
        let parsed = serde_json::from_value::<DiagnosticCaptureRequest>(serde_json::json!({
            "source": "frontend",
            "component": "webview",
            "severity": "error",
            "errorCode": "WEBVIEW_ERROR",
            "details": {"unbounded": "payload"}
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn sensitive_material_is_removed_before_persistence() {
        let state = state();
        state
            .capture(request(
                "password=hunter2 Bearer abcdefghijklmnopqrstuvwxyz /home/alice/work/x.rs \
                 C:\\Users\\alice\\secret.txt file:///tmp/key COM20 libusb:0:1 \
                 serial=ABC123 'ssh root@host dangerous-command' \
                 10.0.0.158 02:11:22:33:44:55 dev@example.com",
            ))
            .unwrap();
        let reports = load_pending(&state, 10, false).unwrap();
        let encoded = serde_json::to_string(&reports).unwrap();
        for forbidden in [
            "hunter2",
            "abcdefghijklmnopqrstuvwxyz",
            "/home/alice",
            "C:\\Users\\alice",
            "file:///tmp/key",
            "COM20",
            "libusb:0:1",
            "ABC123",
            "dangerous-command",
            "10.0.0.158",
            "02:11:22:33:44:55",
            "dev@example.com",
            "raw-user-session-id",
        ] {
            assert!(!encoded.contains(forbidden), "leaked {forbidden}");
        }
        assert!(encoded.contains("[REDACTED]"));
        assert!(encoded.contains("<PATH>"));
        assert!(encoded.contains("<IP>"));
    }

    #[test]
    fn invalid_panic_marker_does_not_block_startup() {
        let path = std::env::temp_dir().join(format!(
            "lynx-invalid-diagnostic-marker-{:032x}.json",
            rand::random::<u128>()
        ));
        std::fs::write(&path, b"not-json").unwrap();
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            "https://example.invalid/api/v1/lynx/diagnostics",
            &path,
        )
        .expect("invalid marker must degrade without blocking startup");
        assert_eq!(
            state.status().unwrap().last_error_code.as_deref(),
            Some("DIAGNOSTIC_PANIC_MARKER_INVALID")
        );
        assert!(!path.exists());
    }

    #[test]
    fn unavailable_database_degrades_without_blocking_startup() {
        let directory = std::env::temp_dir().join(format!(
            "lynx-diagnostic-db-directory-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let marker = directory.join("missing-marker.json");
        let state = DiagnosticState::open(&directory, &marker)
            .expect("diagnostic persistence failure must not block LYNX startup");
        assert_eq!(
            state.status().unwrap().last_error_code.as_deref(),
            Some("DIAGNOSTIC_LOCAL_STORAGE_DEGRADED")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn duplicate_fingerprint_is_aggregated_and_queue_is_bounded() {
        let state = state();
        let first = state.capture(request("render failed")).unwrap();
        let second = state.capture(request("render failed")).unwrap();
        assert_eq!(first.report_id, second.report_id);
        let reports = load_pending(&state, 10, false).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].occurrence_count, 2);

        for index in 0..(MAX_PENDING_REPORTS + 10) {
            state
                .capture(request(&format!("different failure {index}")))
                .unwrap();
        }
        assert_eq!(state.status().unwrap().pending_count, MAX_PENDING_REPORTS);
    }

    #[tokio::test]
    async fn server_ack_only_removes_explicitly_accepted_reports() {
        static REQUESTS: AtomicUsize = AtomicUsize::new(0);
        async fn accept(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
            REQUESTS.fetch_add(1, Ordering::SeqCst);
            let first = body["reports"][0]["reportId"].as_str().unwrap();
            Json(serde_json::json!({"acceptedReportIds":[first]}))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/reports/batch", post(accept)),
            )
            .await
            .unwrap();
        });
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            &format!("http://{address}"),
            Path::new("/path/that/does/not/exist"),
        )
        .unwrap();
        state.capture(request("first failure")).unwrap();
        state.capture(request("second failure")).unwrap();
        let reports = load_pending(&state, 20, false).unwrap();
        let accepted = post_batch(&state, "test-token", "test-instance", &reports)
            .await
            .unwrap();
        acknowledge_reports(&state, &accepted).unwrap();
        assert_eq!(REQUESTS.load(Ordering::SeqCst), 1);
        assert_eq!(state.status().unwrap().pending_count, 1);
    }

    #[tokio::test]
    async fn unauthorized_response_is_retryable_after_login_recovers() {
        async fn unauthorized() -> StatusCode {
            StatusCode::UNAUTHORIZED
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/reports/batch", post(unauthorized)),
            )
            .await
            .unwrap();
        });
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            &format!("http://{address}"),
            Path::new("/path/that/does/not/exist"),
        )
        .unwrap();
        state.capture(request("authentication expired")).unwrap();
        let reports = load_pending(&state, 20, false).unwrap();
        assert_eq!(
            post_batch(&state, "expired-token", "test-instance", &reports)
                .await
                .unwrap_err(),
            "AUTH_REQUIRED"
        );
        let db = state.db.lock().unwrap();
        let (attempts, next_attempt_at): (i64, i64) = db
            .query_row(
                "SELECT attempts,next_attempt_at FROM diagnostic_reports LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts, 1);
        assert!(next_attempt_at > now_seconds());
        assert!(next_attempt_at < i64::MAX / 2);
    }

    #[tokio::test]
    async fn forbidden_rate_limit_and_server_errors_keep_reports_for_retry() {
        async fn controlled_error(headers: HeaderMap) -> StatusCode {
            match headers
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
            {
                Some("Bearer forbidden") => StatusCode::FORBIDDEN,
                Some("Bearer rate-limited") => StatusCode::TOO_MANY_REQUESTS,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/reports/batch", post(controlled_error)),
            )
            .await
            .unwrap();
        });
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            &format!("http://{address}"),
            Path::new("/path/that/does/not/exist"),
        )
        .unwrap();
        state.capture(request("retryable response")).unwrap();
        let reports = load_pending(&state, 20, false).unwrap();

        for (token, expected) in [
            ("forbidden", "AUTH_REQUIRED"),
            ("rate-limited", "RATE_LIMITED"),
            ("server-error", "DIAGNOSTIC_SERVER_ERROR"),
        ] {
            assert_eq!(
                post_batch(&state, token, "test-instance", &reports)
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(state.status().unwrap().pending_count, 1);
        }
    }

    #[tokio::test]
    async fn request_timeout_keeps_report_for_retry() {
        async fn never_in_time() -> StatusCode {
            tokio::time::sleep(Duration::from_secs(2)).await;
            StatusCode::OK
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/reports/batch", post(never_in_time)),
            )
            .await
            .unwrap();
        });
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            &format!("http://{address}"),
            Path::new("/path/that/does/not/exist"),
        )
        .unwrap();
        state.capture(request("request timeout")).unwrap();
        let reports = load_pending(&state, 20, false).unwrap();

        assert_eq!(
            post_batch(&state, "test-token", "test-instance", &reports)
                .await
                .unwrap_err(),
            "NETWORK_UNAVAILABLE"
        );
        assert_eq!(state.status().unwrap().pending_count, 1);
    }

    #[tokio::test]
    async fn duplicate_ack_ids_delete_only_the_requested_report() {
        async fn duplicate_ack(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
            let first = body["reports"][0]["reportId"].as_str().unwrap();
            Json(serde_json::json!({"acceptedReportIds":[first, first]}))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/reports/batch", post(duplicate_ack)),
            )
            .await
            .unwrap();
        });
        let state = DiagnosticState::open_with_base(
            Path::new(":memory:"),
            &format!("http://{address}"),
            Path::new("/path/that/does/not/exist"),
        )
        .unwrap();
        state.capture(request("duplicate acknowledgement")).unwrap();
        let reports = load_pending(&state, 20, false).unwrap();

        let accepted = post_batch(&state, "test-token", "test-instance", &reports)
            .await
            .unwrap();
        acknowledge_reports(&state, &accepted).unwrap();
        assert_eq!(state.status().unwrap().pending_count, 0);
    }

    #[test]
    fn panic_marker_contains_no_source_path_or_message() {
        let path = std::env::temp_dir().join(format!(
            "lynx-diagnostic-marker-{:032x}.json",
            rand::random::<u128>()
        ));
        configure_panic_marker(path.clone());
        let fingerprint = record_panic_marker(None);
        let marker = std::fs::read_to_string(&path).unwrap();
        assert!(marker.contains(&fingerprint));
        assert!(!marker.contains("panic message"));
        assert!(!marker.contains("/home/"));
        let _ = std::fs::remove_file(path);
    }
}

//! Local MCP gateway for AI coding agents.
//!
//! The gateway intentionally binds to loopback only. Remote Codex or Claude
//! clients reach it through an authenticated SSH reverse tunnel, so hardware
//! capabilities are never exposed directly on the LAN.

mod connection_status;
mod rockchip;
pub use connection_status::GatewayChannelStatus;
mod router;

use rockchip::{RockchipLoaderRequest, RockchipLoaderTransition};

use router::gateway_router;

use std::{
    collections::{HashMap, VecDeque},
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

use rmcp::{
    handler::server::tool::ToolRouter,
    model::{CallToolResult, ContentBlock as Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::app_error::AppError;
use crate::skills::{SkillDomain, SkillGetRequest, SkillReadRequest, SkillSearchRequest};

const DEFAULT_PORT: u16 = 8765;
const SERIAL_MCP_QUIET_WINDOW: Duration = Duration::from_millis(12);
const SERIAL_MCP_POLL_INTERVAL: Duration = Duration::from_millis(4);
const MAX_CAMERA_JPEG_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug)]
struct GatewayRuntime {
    cancellation_token: CancellationToken,
    channel_scope: Option<ChannelScope>,
    generation: u64,
}

/// Process-wide MCP gateway lifecycle state.
pub struct GatewayState {
    runtimes: Arc<Mutex<HashMap<u16, GatewayRuntime>>>,
    interactions: Arc<Mutex<VecDeque<GatewayInteraction>>>,
    next_interaction_id: Arc<AtomicU64>,
    next_runtime_generation: AtomicU64,
}

impl Default for GatewayState {
    fn default() -> Self {
        Self {
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            interactions: Arc::new(Mutex::new(VecDeque::with_capacity(200))),
            next_interaction_id: Arc::new(AtomicU64::new(0)),
            next_runtime_generation: AtomicU64::new(0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayInteraction {
    pub id: u64,
    pub timestamp_ms: u64,
    pub tool: String,
    pub category: String,
    pub summary: String,
    pub device_key: String,
    pub project_id: Option<i64>,
    pub device_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub port: u16,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayNetworkInfo {
    pub host_name: String,
    pub local_ip: String,
    pub loopback: String,
}

impl GatewayStatus {
    fn stopped(port: u16) -> Self {
        Self {
            running: false,
            port,
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
        }
    }

    fn running(port: u16) -> Self {
        Self {
            running: true,
            port,
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
        }
    }
}

#[derive(Clone, Default)]
struct SerialSessions {
    ports: Arc<Mutex<HashMap<String, Box<dyn serialport::SerialPort>>>>,
    handle_ports: Arc<Mutex<HashMap<String, String>>>,
    handle_bauds: Arc<Mutex<HashMap<String, u32>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct LynxMcp {
    tool_router: ToolRouter<Self>,
    app_handle: Option<AppHandle>,
    serial: SerialSessions,
    interactions: Arc<Mutex<VecDeque<GatewayInteraction>>>,
    next_interaction_id: Arc<AtomicU64>,
    channel_scope: Option<ChannelScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelScope {
    project_id: i64,
    project_name: String,
    mcp_server_name: String,
    device_id: i64,
    chip_vendor: String,
    flasher_kind: String,
    flasher_path: String,
    serial_port: String,
    usb_binding: String,
    adb_serial: String,
    camera_binding: String,
}

fn same_gateway_route(current: &Option<ChannelScope>, requested: &Option<ChannelScope>) -> bool {
    match (current, requested) {
        (None, None) => true,
        (Some(current), Some(requested)) => current.project_id == requested.project_id,
        _ => false,
    }
}

fn live_serial_port(device: &crate::workbench::types::DeviceConfig) -> String {
    crate::serial::commands::resolve_port_core(&crate::serial::commands::SerialIdentity {
        port: device.serial_port.clone(),
        vid: device.serial_vid,
        pid: device.serial_pid,
        serial_number: device.serial_number.clone(),
        interface_number: device.serial_interface_number,
    })
    .map(|port| port.name)
    .unwrap_or_else(|_| device.serial_port.clone())
}

impl LynxMcp {
    #[cfg(test)]
    fn new(
        app_handle: Option<AppHandle>,
        interactions: Arc<Mutex<VecDeque<GatewayInteraction>>>,
        next_interaction_id: Arc<AtomicU64>,
        channel_scope: Option<ChannelScope>,
    ) -> Self {
        Self::with_serial(
            app_handle,
            interactions,
            next_interaction_id,
            channel_scope,
            SerialSessions::default(),
        )
    }

    fn with_serial(
        app_handle: Option<AppHandle>,
        interactions: Arc<Mutex<VecDeque<GatewayInteraction>>>,
        next_interaction_id: Arc<AtomicU64>,
        channel_scope: Option<ChannelScope>,
        serial: SerialSessions,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            app_handle,
            serial,
            interactions,
            next_interaction_id,
            channel_scope,
        }
    }

    fn serial_binding(&self, handle: &str) -> Result<(String, u32), String> {
        let port = self
            .serial
            .handle_ports
            .lock()
            .map_err(|error| error.to_string())?
            .get(handle)
            .cloned()
            .ok_or_else(|| format!("SERIAL_HANDLE_EXPIRED:{handle}:call lynx_serial_open"))?;
        let baud = self
            .serial
            .handle_bauds
            .lock()
            .map_err(|error| error.to_string())?
            .get(handle)
            .copied()
            .ok_or_else(|| format!("SERIAL_HANDLE_EXPIRED:{handle}:call lynx_serial_open"))?;
        Ok((port, baud))
    }

    /// Resolve the latest saved board binding for every MCP request. Long-lived
    /// Codex/Claude sessions must see interface edits immediately without
    /// requiring the user to remove and re-add the remote MCP server.
    fn live_channel_scope(&self) -> Option<ChannelScope> {
        let configured = self.channel_scope.as_ref()?.clone();
        let Some(app_handle) = self.app_handle.as_ref() else {
            return Some(configured);
        };
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let Ok(db) = workbench.db.lock() else {
            return Some(configured);
        };
        let Ok(project) = crate::workbench::storage::project_by_id(&db, configured.project_id)
        else {
            return Some(configured);
        };
        let Some(device_id) = project.device_id else {
            return Some(configured);
        };
        let Ok(device) = crate::workbench::storage::device_by_id(&db, device_id) else {
            return Some(configured);
        };
        drop(db);
        let serial_port = live_serial_port(&device);
        Some(ChannelScope {
            project_id: configured.project_id,
            project_name: project.name,
            mcp_server_name: project.mcp_server_name,
            device_id,
            chip_vendor: device.chip_vendor,
            flasher_kind: device.flasher_kind,
            flasher_path: device.flasher_path,
            serial_port,
            usb_binding: device.usb_binding,
            adb_serial: device.adb_serial,
            camera_binding: device.camera_binding,
        })
    }

    fn ensure_hosted_serial_open(
        &self,
        app_handle: &AppHandle,
        port: &str,
        baud_rate: u32,
    ) -> Result<(), String> {
        let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
        if crate::serial::commands::serial_is_open_core(serial_state.inner().as_ref(), port) {
            return Ok(());
        }
        let _ = crate::serial::commands::serial_close_core(serial_state.inner().as_ref(), port);
        crate::serial::commands::serial_open_core(
            serial_state.inner().as_ref(),
            app_handle.clone(),
            port.to_owned(),
            baud_rate,
            8,
            1,
            "none".to_owned(),
            "none".to_owned(),
            None,
        )
        .map_err(|error| format!("SERIAL_RECONNECT_FAILED:{port}:{error}"))
    }

    fn record_interaction(&self, tool: &str, category: &str, summary: impl Into<String>) {
        self.record_device_interaction(tool, category, summary, "");
    }

    fn ensure_serial_scope(&self, port: &str) -> Result<(), String> {
        if let Some(scope) = self.live_channel_scope() {
            if scope.serial_port != port {
                return Err(format!(
                    "DEVICE_SCOPE:该 AI 通道只允许访问单板 {} 的串口 {}",
                    scope.device_id, scope.serial_port
                ));
            }
        }
        Ok(())
    }

    fn ensure_task_scope(&self, task: &crate::workbench::types::TaskHistory) -> Result<(), String> {
        let Some(scope) = self.live_channel_scope() else {
            return Ok(());
        };
        let belongs = match task.project_id {
            Some(project_id) => project_id == scope.project_id,
            None => task.device_id == Some(scope.device_id),
        };
        if belongs {
            Ok(())
        } else {
            Err(format!(
                "CHANNEL_SCOPE:任务 {} 不属于当前 LYNX SDK 通道",
                task.task_id
            ))
        }
    }

    fn record_device_interaction(
        &self,
        tool: &str,
        category: &str,
        summary: impl Into<String>,
        device_key: impl Into<String>,
    ) {
        let live_scope = self.live_channel_scope();
        let requested_device_key = device_key.into();
        let device_key = if requested_device_key.is_empty() {
            live_scope
                .as_ref()
                .map(|scope| format!("project:{}:device:{}", scope.project_id, scope.device_id))
                .unwrap_or_default()
        } else {
            requested_device_key
        };
        let interaction = GatewayInteraction {
            id: self.next_interaction_id.fetch_add(1, Ordering::SeqCst) + 1,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            tool: tool.to_owned(),
            category: category.to_owned(),
            summary: summary.into(),
            device_key,
            project_id: live_scope.as_ref().map(|scope| scope.project_id),
            device_id: live_scope.as_ref().map(|scope| scope.device_id),
        };
        if let Ok(mut items) = self.interactions.lock() {
            if items.len() >= 200 {
                items.pop_front();
            }
            items.push_back(interaction.clone());
        }
        if let Some(app_handle) = &self.app_handle {
            let _ = app_handle.emit("gateway-interaction", interaction);
        }
    }
}

fn task_status_value(task: &crate::workbench::types::TaskHistory) -> Value {
    let mut value = json!({
        "taskId": task.task_id,
        "kind": task.kind,
        "state": task.status,
        "phase": task.phase,
        "progressPct": task.progress_pct,
        "projectId": task.project_id,
        "deviceId": task.device_id,
        "startedAt": task.started_at,
        "finishedAt": task.finished_at,
        "exitCode": task.exit_code,
        "error": if task.error.is_empty() { Value::Null } else { Value::String(task.error.clone()) },
    });
    if let (Some(target), Some(details)) = (
        value.as_object_mut(),
        task.details.as_ref().and_then(Value::as_object),
    ) {
        for (key, item) in details {
            if !matches!(key.as_str(), "phase" | "progressPct") {
                target.insert(key.clone(), item.clone());
            }
        }
    }
    value
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InspectFirmwareRequest {
    /// Absolute path to a vendor firmware image on the LYNX host.
    image_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct DownloadFirmwareRequest {
    /// Saved SDK channel project ID. May be omitted for a device-scoped Gateway
    /// when exactly one saved project belongs to that device.
    project_id: Option<i64>,
    /// Absolute firmware path selected by the remote AI on the SDK host.
    remote_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct WorkbenchTaskRequest {
    /// Task ID returned by lynx_download_firmware.
    task_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct StartFlashRequest {
    /// Absolute path to the vendor firmware package on the LYNX host.
    firmware_path: String,
    /// The AI's in-band acknowledgement that the selected operation is destructive.
    confirm_destructive: bool,
    /// Optional backend-neutral and vendor-specific choices. Query lynx_flash_capabilities first.
    options: Option<FlashOptionsRequest>,
    /// Explicitly allow a large Allwinner image over a USB Full Speed FEL link.
    /// This is intentionally separate from confirmDestructive because the transfer
    /// may be slow or time out even when the selected target and image are correct.
    #[serde(default)]
    allow_full_speed_large_image: bool,
    /// Optional fixed Allwinner transition recipe. When present, LYNX enters
    /// FEL and starts flashing under one uninterrupted hardware guard.
    allwinner_enter_fel: Option<AllwinnerFelTransitionRequest>,
    /// Fixed Rockchip normal-to-Loader recipe; shares the flash hardware guard.
    rockchip_enter_loader: Option<RockchipLoaderTransition>,
}

const MAINLINE_ARTIFACT_LIMIT: usize = 8;
const MAINLINE_ARTIFACT_MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAINLINE_PLAN_MAX_BYTES: u64 = 768 * 1024 * 1024;
const MAINLINE_OPEN_TIMEOUT: Duration = Duration::from_secs(20);
const MAINLINE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const MAINLINE_WORKER_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const MAINLINE_OUTPUT_LINE_LIMIT: usize = 64 * 1024;
const R528_MAINLINE_SPL_ADDRESS: u32 = 0x0002_0000;
const R528_MAINLINE_SPL_MAX_BYTES: usize = 160 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MainlineArtifactRole {
    Spl,
    Bootloader,
    Kernel,
    DeviceTree,
    Initramfs,
    TrustedFirmware,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MainlineArtifactRequest {
    /// Enumerated artifact purpose; arbitrary executable operation names are not accepted.
    role: MainlineArtifactRole,
    /// Absolute path on the desktop-hosted LYNX machine.
    file_path: String,
    /// Four-byte-aligned FEL memory address selected for this exact board/SoC.
    load_address: u32,
    /// Required lowercase or uppercase SHA-256 digest of the complete file.
    sha256: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AllwinnerMainlineBootRequest {
    /// Ordered mainline artifacts. Exactly one `spl` built by mainline U-Boot
    /// for R528/T113-S3 must precede all DRAM artifacts.
    artifacts: Vec<MainlineArtifactRequest>,
    /// Entry address to execute; it must equal one artifact's loadAddress.
    entry_address: u32,
    /// Explicit acknowledgement that loading and executing board code is destructive/high risk.
    confirm_destructive: bool,
}

#[derive(Debug)]
struct ValidatedMainlineArtifact {
    role: MainlineArtifactRole,
    file_path: String,
    load_address: u32,
    sha256: String,
    size_bytes: u64,
}

fn ensure_mainline_not_cancelled(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Relaxed) {
        Err("TASK_CANCELLED:mainline FEL boot cancelled by operator".to_owned())
    } else {
        Ok(())
    }
}

fn insert_mainline_task(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    device_id: i64,
    artifact_count: usize,
    total_bytes: u64,
) -> Result<(), String> {
    let db = workbench.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO task_history(task_id,kind,device_id,status,started_at,phase,progress,details_json) \
         VALUES(?1,'mainline_boot',?2,'running',unixepoch(),'queued',0,?3)",
        rusqlite::params![
            task_id,
            device_id,
            json!({
                "artifactCount": artifact_count,
                "totalBytes": total_bytes,
                "writtenBytes": 0,
            })
            .to_string()
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn update_mainline_task_progress(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    phase: &str,
    written_bytes: u64,
    total_bytes: u64,
) {
    let progress = if total_bytes == 0 {
        0.0
    } else {
        // FEL only stages the installer in RAM. Reserve the second half of
        // the UI progress bar for the installer-confirmed NAND operation.
        (written_bytes as f64 / total_bytes as f64 * 50.0).clamp(0.0, 50.0)
    };
    if let Ok(db) = workbench.db.lock() {
        let _ = db.execute(
            "UPDATE task_history SET phase=?1,progress=?2,details_json=?3 \
             WHERE task_id=?4 AND status='running'",
            rusqlite::params![
                phase,
                progress,
                json!({
                    "writtenBytes": written_bytes,
                    "totalBytes": total_bytes,
                })
                .to_string(),
                task_id,
            ],
        );
    }
}

fn update_mainline_installer_progress(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    phase: &str,
    progress: f64,
    partition: &str,
) {
    if let Ok(db) = workbench.db.lock() {
        let _ = db.execute(
            "UPDATE task_history SET phase=?1,progress=?2,details_json=?3 \
             WHERE task_id=?4 AND status='running'",
            rusqlite::params![
                phase,
                progress.clamp(50.0, 100.0),
                json!({
                    "progressPct": progress.clamp(50.0, 100.0),
                    "currentPartition": partition,
                    "mediaStage": "nand_install",
                })
                .to_string(),
                task_id,
            ],
        );
    }
}

fn emit_mainline_flash_log(
    app: &AppHandle,
    task_id: &str,
    stream: &str,
    message: impl Into<String>,
) {
    // The flash console is a protocol console, not a serial mirror. UART is
    // still consumed by the installer monitor for structured progress markers
    // and continues to flow through the dedicated serial terminal.
    if !mainline_flash_log_stream_allowed(stream) {
        return;
    }
    let _ = app.emit(
        "workbench-task-event",
        crate::workbench::types::TaskEvent {
            task_id: task_id.to_owned(),
            phase: "flash".to_owned(),
            stream: stream.to_owned(),
            message: message.into(),
            current: None,
            total: None,
            status: "running".to_owned(),
        },
    );
}

fn mainline_flash_log_stream_allowed(stream: &str) -> bool {
    matches!(stream, "fel" | "stderr")
}

fn monitor_mainline_installer(
    app: &AppHandle,
    serial_port: &str,
    cancellation: &AtomicBool,
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
) -> Result<(), String> {
    if serial_port.trim().is_empty() {
        return Err("MAINLINE_INSTALLER_SERIAL_UNBOUND".to_owned());
    }
    let serial = app.state::<Arc<crate::serial::SerialState>>();
    if !crate::serial::commands::serial_is_open_core(serial.inner().as_ref(), serial_port) {
        crate::serial::commands::serial_open_core(
            serial.inner().as_ref(),
            app.clone(),
            serial_port.to_owned(),
            115_200,
            8,
            1,
            "none".to_owned(),
            "none".to_owned(),
            None,
        )?;
    }

    update_mainline_installer_progress(
        workbench,
        task_id,
        "waiting_for_installer",
        50.0,
        "payload",
    );
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut pending = String::new();
    while Instant::now() < deadline {
        ensure_mainline_not_cancelled(cancellation)?;
        let chunk = crate::serial::commands::serial_read_core(
            serial.inner().as_ref(),
            serial_port,
            65_536,
        )?;
        if chunk.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = pending.find('\n') {
            let line = pending[..newline].trim_matches(['\r', ' ']).to_owned();
            pending.drain(..=newline);
            let Some(marker) = line.strip_prefix("LYNX_PROGRESS ") else {
                continue;
            };
            let mut phase = "waiting_for_installer";
            let mut progress = 50.0;
            let mut partition = "unknown";
            for field in marker.split_ascii_whitespace() {
                if let Some(value) = field.strip_prefix("phase=") {
                    phase = value;
                } else if let Some(value) = field.strip_prefix("progress=") {
                    progress = value.parse::<f64>().unwrap_or(progress);
                } else if let Some(value) = field.strip_prefix("partition=") {
                    partition = value;
                }
            }
            update_mainline_installer_progress(workbench, task_id, phase, progress, partition);
            if phase == "installer_complete" {
                return Ok(());
            }
            if phase == "installer_failed" {
                return Err(format!("MAINLINE_INSTALLER_FAILED:partition={partition}"));
            }
        }
        if pending.len() > 8192 {
            pending.drain(..pending.len() - 8192);
        }
    }
    Err("MAINLINE_INSTALLER_TIMEOUT:no completion marker within 180 seconds".to_owned())
}

fn finish_mainline_task(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    status: &str,
    error: &str,
) {
    if let Ok(db) = workbench.db.lock() {
        let _ = crate::workbench::storage::transition_task_status(
            &db,
            task_id,
            status,
            (status == "success").then_some(0),
            error,
        );
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn mainline_u32_le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "MAINLINE_SPL_HEADER_TRUNCATED".to_owned())
}

fn validate_r528_mainline_spl(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 0x20 || bytes.get(4..12) != Some(b"eGON.BT0") {
        return Err("MAINLINE_SPL_EGON_REQUIRED".to_owned());
    }
    if bytes.get(0x14..0x17) != Some(b"SPL") {
        return Err("MAINLINE_SPL_SIGNATURE_REQUIRED:vendor Boot0 is not accepted".to_owned());
    }
    let version = bytes[0x17];
    if !(1..=31).contains(&version) {
        return Err(format!("MAINLINE_SPL_VERSION_UNSUPPORTED:{version}"));
    }
    let length = usize::try_from(mainline_u32_le(bytes, 0x10)?)
        .map_err(|_| "MAINLINE_SPL_LENGTH_OVERFLOW".to_owned())?;
    if !(0x20..=R528_MAINLINE_SPL_MAX_BYTES).contains(&length) || !length.is_multiple_of(4) {
        return Err(format!("MAINLINE_SPL_LENGTH_INVALID:{length}"));
    }
    if length != bytes.len() {
        return Err(format!(
            "MAINLINE_SPL_EXACT_FILE_REQUIRED:header={length}:file={}",
            bytes.len()
        ));
    }
    let stored_checksum = mainline_u32_le(bytes, 0x0c)?;
    let expected_sum = stored_checksum.wrapping_mul(2).wrapping_sub(0x5f0a_6c39);
    let actual_sum = bytes.as_chunks::<4>().0.iter().fold(0_u32, |sum, word| {
        sum.wrapping_add(u32::from_le_bytes(*word))
    });
    if actual_sum != expected_sum {
        return Err("MAINLINE_SPL_CHECKSUM_INVALID".to_owned());
    }
    Ok(())
}

fn validate_mainline_artifact(
    request: MainlineArtifactRequest,
) -> Result<ValidatedMainlineArtifact, String> {
    if request.load_address < 0x1_000 || !request.load_address.is_multiple_of(4) {
        return Err(format!(
            "MAINLINE_LOAD_ADDRESS_INVALID:0x{:08x}",
            request.load_address
        ));
    }
    if !valid_sha256(request.sha256.trim()) {
        return Err("MAINLINE_SHA256_INVALID:expected 64 hexadecimal characters".to_owned());
    }
    let path = Path::new(&request.file_path);
    if !path.is_absolute() {
        return Err("MAINLINE_ARTIFACT_PATH_INVALID:absolute path required".to_owned());
    }
    let metadata = std::fs::metadata(path).map_err(|_| "MAINLINE_ARTIFACT_NOT_FOUND".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("MAINLINE_ARTIFACT_EMPTY_OR_NOT_FILE".to_owned());
    }
    if metadata.len() > MAINLINE_ARTIFACT_MAX_BYTES {
        return Err(format!(
            "MAINLINE_ARTIFACT_TOO_LARGE:{}>{MAINLINE_ARTIFACT_MAX_BYTES}",
            metadata.len()
        ));
    }
    let end = u64::from(request.load_address)
        .checked_add(metadata.len())
        .filter(|end| *end <= u64::from(u32::MAX) + 1)
        .ok_or_else(|| "MAINLINE_ADDRESS_RANGE_OVERFLOW".to_owned())?;
    if end <= u64::from(request.load_address) {
        return Err("MAINLINE_ADDRESS_RANGE_OVERFLOW".to_owned());
    }
    let mut file =
        std::fs::File::open(path).map_err(|_| "MAINLINE_ARTIFACT_READ_FAILED".to_owned())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut spl_bytes = matches!(request.role, MainlineArtifactRole::Spl).then(Vec::new);
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "MAINLINE_ARTIFACT_READ_FAILED".to_owned())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if let Some(spl_bytes) = spl_bytes.as_mut() {
            spl_bytes.extend_from_slice(&buffer[..read]);
        }
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(request.sha256.trim()) {
        return Err("MAINLINE_ARTIFACT_HASH_MISMATCH".to_owned());
    }
    if let Some(spl_bytes) = spl_bytes.as_deref() {
        validate_r528_mainline_spl(spl_bytes)?;
    }
    Ok(ValidatedMainlineArtifact {
        role: request.role,
        file_path: request.file_path,
        load_address: request.load_address,
        sha256: actual,
        size_bytes: metadata.len(),
    })
}

fn validate_mainline_plan(
    request: AllwinnerMainlineBootRequest,
) -> Result<(Vec<ValidatedMainlineArtifact>, u32), String> {
    if !request.confirm_destructive {
        return Err(
            "CONFIRMATION_REQUIRED:confirmDestructive=true is required for mainline FEL execution"
                .to_owned(),
        );
    }
    if request.artifacts.is_empty() || request.artifacts.len() > MAINLINE_ARTIFACT_LIMIT {
        return Err(format!(
            "MAINLINE_ARTIFACT_COUNT_INVALID:expected 1..={MAINLINE_ARTIFACT_LIMIT}"
        ));
    }
    let spl_requests = request
        .artifacts
        .iter()
        .filter(|artifact| matches!(artifact.role, MainlineArtifactRole::Spl))
        .collect::<Vec<_>>();
    if spl_requests.len() != 1 {
        return Err(
            "ALLWINNER_MAINLINE_SPL_REQUIRED:expected exactly one mainline U-Boot SPL".to_owned(),
        );
    }
    if !matches!(request.artifacts[0].role, MainlineArtifactRole::Spl) {
        return Err("MAINLINE_SPL_MUST_BE_FIRST".to_owned());
    }
    if spl_requests[0].load_address != R528_MAINLINE_SPL_ADDRESS {
        return Err(format!(
            "MAINLINE_SPL_ADDRESS_INVALID:expected 0x{R528_MAINLINE_SPL_ADDRESS:08x}"
        ));
    }
    if request.entry_address == R528_MAINLINE_SPL_ADDRESS {
        return Err(
            "MAINLINE_ENTRY_SPL_FORBIDDEN:entry must be U-Boot proper or installer".to_owned(),
        );
    }
    let bootloaders = request
        .artifacts
        .iter()
        .filter(|artifact| matches!(artifact.role, MainlineArtifactRole::Bootloader))
        .collect::<Vec<_>>();
    if bootloaders.len() != 1 {
        return Err("MAINLINE_BOOTLOADER_REQUIRED:expected exactly one U-Boot proper".to_owned());
    }
    if bootloaders[0].load_address != request.entry_address {
        return Err("MAINLINE_ENTRY_MUST_BE_BOOTLOADER".to_owned());
    }
    let mut artifacts = Vec::with_capacity(request.artifacts.len());
    let mut total_bytes = 0_u64;
    for artifact in request.artifacts {
        let artifact = validate_mainline_artifact(artifact)?;
        total_bytes = total_bytes
            .checked_add(artifact.size_bytes)
            .filter(|total| *total <= MAINLINE_PLAN_MAX_BYTES)
            .ok_or_else(|| "MAINLINE_PLAN_TOO_LARGE".to_owned())?;
        let start = u64::from(artifact.load_address);
        let end = start + artifact.size_bytes;
        if artifacts
            .iter()
            .any(|existing: &ValidatedMainlineArtifact| {
                let existing_start = u64::from(existing.load_address);
                let existing_end = existing_start + existing.size_bytes;
                start < existing_end && existing_start < end
            })
        {
            return Err("MAINLINE_ARTIFACT_ADDRESS_OVERLAP".to_owned());
        }
        artifacts.push(artifact);
    }
    if !artifacts
        .iter()
        .any(|artifact| artifact.load_address == request.entry_address)
    {
        return Err("MAINLINE_ENTRY_NOT_DECLARED".to_owned());
    }
    Ok((artifacts, request.entry_address))
}

struct MainlineWorkerFiles {
    plan: std::path::PathBuf,
    cancel: std::path::PathBuf,
}

impl MainlineWorkerFiles {
    fn create(
        task_id: &str,
        artifacts: &[ValidatedMainlineArtifact],
        entry_address: u32,
    ) -> Result<Self, String> {
        let safe_task = task_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let root = std::env::temp_dir();
        let nonce = rand::random::<u64>();
        let plan = root.join(format!("lynx-mainline-{safe_task}-{nonce:016x}.json"));
        let cancel = root.join(format!("lynx-mainline-{safe_task}-{nonce:016x}.cancel"));
        let payload = json!({
            "artifacts": artifacts.iter().map(|artifact| json!({
                "role": artifact.role,
                "filePath": artifact.file_path,
                "loadAddress": artifact.load_address,
                "sha256": artifact.sha256,
            })).collect::<Vec<_>>(),
            "entryAddress": entry_address,
        });
        let encoded = serde_json::to_vec(&payload)
            .map_err(|error| format!("MAINLINE_PLAN_ENCODE:{error}"))?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&plan)
            .map_err(|_| "MAINLINE_PLAN_CREATE_FAILED".to_owned())?;
        file.write_all(&encoded)
            .map_err(|_| "MAINLINE_PLAN_WRITE_FAILED".to_owned())?;
        file.sync_all()
            .map_err(|_| "MAINLINE_PLAN_SYNC_FAILED".to_owned())?;
        Ok(Self { plan, cancel })
    }

    fn request_cancel(&self) -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&self.cancel)
            .and_then(|mut file| file.write_all(b"cancel"))
            .map_err(|_| "MAINLINE_CANCEL_SIGNAL_FAILED".to_owned())
    }
}

impl Drop for MainlineWorkerFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.plan);
        let _ = std::fs::remove_file(&self.cancel);
    }
}

fn spawn_mainline_reader<R: Read + Send + 'static>(
    reader: R,
    stream: &'static str,
    sender: mpsc::Sender<(&'static str, String)>,
) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0_u8; 8192];
        let mut line = Vec::new();
        let mut truncated = false;
        loop {
            let read = match reader.read(&mut chunk) {
                Ok(0) => {
                    if !line.is_empty() || truncated {
                        if truncated {
                            line.extend_from_slice(b"...[truncated]");
                        }
                        let _ = sender.send((stream, String::from_utf8_lossy(&line).into_owned()));
                    }
                    break;
                }
                Ok(read) => read,
                Err(_) => break,
            };
            for byte in &chunk[..read] {
                if *byte == b'\n' {
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if truncated {
                        line.extend_from_slice(b"...[truncated]");
                    }
                    if sender
                        .send((stream, String::from_utf8_lossy(&line).into_owned()))
                        .is_err()
                    {
                        return;
                    }
                    line.clear();
                    truncated = false;
                } else if line.len() < MAINLINE_OUTPUT_LINE_LIMIT {
                    line.push(*byte);
                } else {
                    truncated = true;
                }
            }
        }
    });
}

fn wait_mainline_worker_exit(child: &mut std::process::Child) -> Result<bool, String> {
    let deadline = Instant::now() + MAINLINE_WORKER_EXIT_TIMEOUT;
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("MAINLINE_WORKER_WAIT:{error}"))?
        {
            Some(_) => return Ok(true),
            None if Instant::now() >= deadline => return Ok(false),
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn terminate_mainline_worker(
    process_job: &crate::process::ProcessJob,
    child: &mut std::process::Child,
) -> Result<(), String> {
    // Release a pending synchronous CreateFile/driver request before asking
    // Windows to terminate the process tree. This is best-effort and bounded.
    let _ = process_job.cancel_blocking_io();
    std::thread::sleep(Duration::from_millis(100));
    if child
        .try_wait()
        .map_err(|error| format!("MAINLINE_WORKER_WAIT:{error}"))?
        .is_some()
    {
        return Ok(());
    }
    let job_result = process_job.terminate();
    // `Child::kill` is intentionally also attempted. It is the fallback on
    // non-Windows platforms and covers helpers that could not be assigned to
    // the Windows Job Object before termination was requested.
    let child_result = child.kill();
    if !wait_mainline_worker_exit(child)? {
        return Err("MAINLINE_WORKER_KILL_TIMEOUT".to_owned());
    }
    job_result?;
    if let Err(error) = child_result {
        // Killing an already-exited process is harmless; try_wait above is the
        // authoritative confirmation that the helper is gone.
        if child.try_wait().ok().flatten().is_none() {
            return Err(format!("MAINLINE_WORKER_KILL:{error}"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentAllwinnerEndpoint {
    bus: u8,
    port: u8,
}

fn resolve_current_allwinner_endpoint(
    usb_location: &str,
    devices: &[libefex::ScannedDevice],
) -> Result<CurrentAllwinnerEndpoint, String> {
    let matches = devices
        .iter()
        .filter(|device| {
            device.location_path.as_deref().is_some_and(|candidate| {
                crate::workbench::flasher::allwinner_binding_matches(
                    &format!("location:{usb_location}"),
                    &format!("location:{candidate}"),
                )
            })
        })
        .map(|device| CurrentAllwinnerEndpoint {
            bus: device.bus,
            port: device.port,
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [endpoint] => Ok(*endpoint),
        [] => Err(
            "ALLWINNER_FEL_LOCATION_OFFLINE:bound physical location is not currently enumerated"
                .to_owned(),
        ),
        _ => Err(
            "ALLWINNER_FEL_LOCATION_AMBIGUOUS:physical location resolved to multiple current endpoints"
                .to_owned(),
        ),
    }
}

fn scan_current_allwinner_endpoint(usb_location: &str) -> Result<CurrentAllwinnerEndpoint, String> {
    let devices = libefex::Context::scan_usb_devices()
        .map_err(|error| format!("ALLWINNER_FEL_ENDPOINT_SCAN:{error}"))?;
    resolve_current_allwinner_endpoint(usb_location, &devices)
}

fn mainline_error_with_cleanup(primary: &str, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => primary.to_owned(),
        Err(cleanup) => format!("{primary}:cleanup={cleanup}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_mainline_worker(
    app: &AppHandle,
    executable: &Path,
    usb_location: &str,
    endpoint: CurrentAllwinnerEndpoint,
    artifacts: &[ValidatedMainlineArtifact],
    entry_address: u32,
    total_bytes: u64,
    cancellation: &AtomicBool,
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
) -> Result<(), String> {
    run_mainline_worker_with_timeouts(
        Some(app),
        executable,
        usb_location,
        endpoint,
        artifacts,
        entry_address,
        total_bytes,
        cancellation,
        workbench,
        task_id,
        MAINLINE_OPEN_TIMEOUT,
        MAINLINE_PROGRESS_TIMEOUT,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_mainline_worker_with_timeouts(
    app: Option<&AppHandle>,
    executable: &Path,
    usb_location: &str,
    endpoint: CurrentAllwinnerEndpoint,
    artifacts: &[ValidatedMainlineArtifact],
    entry_address: u32,
    total_bytes: u64,
    cancellation: &AtomicBool,
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    open_timeout: Duration,
    progress_timeout: Duration,
) -> Result<(), String> {
    let files = MainlineWorkerFiles::create(task_id, artifacts, entry_address)?;
    let mut command = Command::new(executable);
    command
        .args(["--output", "jsonl", "boot-mainline", "--plan"])
        .arg(&files.plan)
        .args(["--device-location", usb_location])
        .args(["--bus", &endpoint.bus.to_string()])
        .args(["--port", &endpoint.port.to_string()])
        .env("LYNX_FLASH_CANCEL_FILE", &files.cancel);
    supervise_mainline_worker(
        app,
        command,
        &files,
        usb_location,
        total_bytes,
        cancellation,
        workbench,
        task_id,
        open_timeout,
        progress_timeout,
    )
}

#[allow(clippy::too_many_arguments)]
fn supervise_mainline_worker(
    app: Option<&AppHandle>,
    mut command: Command,
    files: &MainlineWorkerFiles,
    usb_location: &str,
    total_bytes: u64,
    cancellation: &AtomicBool,
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    open_timeout: Duration,
    progress_timeout: Duration,
) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("MAINLINE_WORKER_START:{error}"))?;
    let process_job = match crate::process::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = wait_mainline_worker_exit(&mut child);
            return Err(error);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "MAINLINE_WORKER_STDOUT_UNAVAILABLE".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "MAINLINE_WORKER_STDERR_UNAVAILABLE".to_owned())?;
    let (sender, receiver) = mpsc::channel();
    spawn_mainline_reader(stdout, "stdout", sender.clone());
    spawn_mainline_reader(stderr, "stderr", sender);
    let mut phase = "opening_device".to_owned();
    let mut phase_started = Instant::now();
    let mut last_progress = Instant::now();
    let mut stderr_tail = VecDeque::with_capacity(20);
    let mut last_logged_phase = String::new();
    let mut last_logged_percent = None;

    loop {
        if cancellation.load(Ordering::Relaxed) {
            let _ = files.request_cancel();
            let primary = "TASK_CANCELLED:mainline FEL boot cancelled by operator";
            return Err(mainline_error_with_cleanup(
                primary,
                terminate_mainline_worker(&process_job, &mut child),
            ));
        }

        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(("stdout", line)) => {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    let mut observed_progress = false;
                    let mut emitted_line = false;
                    if let Some(next_phase) = value.get("phase").and_then(Value::as_str) {
                        observed_progress = true;
                        if next_phase != phase {
                            phase = next_phase.to_owned();
                            phase_started = Instant::now();
                        }
                    }
                    if let Some(written) = value.get("writtenBytes").and_then(Value::as_u64) {
                        observed_progress = true;
                        let percent = written
                            .saturating_mul(100)
                            .checked_div(total_bytes)
                            .unwrap_or(0);
                        if last_logged_percent != Some(percent) {
                            if let Some(app) = app {
                                emit_mainline_flash_log(app, task_id, "fel", line.clone());
                            }
                            last_logged_percent = Some(percent);
                            emitted_line = true;
                        }
                        update_mainline_task_progress(
                            workbench,
                            task_id,
                            &phase,
                            written,
                            total_bytes,
                        );
                    }
                    if observed_progress {
                        last_progress = Instant::now();
                    }
                    if let Some(next_phase) = value.get("phase").and_then(Value::as_str) {
                        if next_phase != last_logged_phase {
                            if !emitted_line {
                                if let Some(app) = app {
                                    emit_mainline_flash_log(app, task_id, "fel", line.clone());
                                }
                            }
                            last_logged_phase = next_phase.to_owned();
                        }
                    }
                } else {
                    if let Some(app) = app {
                        emit_mainline_flash_log(app, task_id, "fel", line);
                    }
                }
            }
            Ok((_, line)) => {
                if let Some(app) = app {
                    emit_mainline_flash_log(app, task_id, "stderr", line.clone());
                }
                if stderr_tail.len() == 20 {
                    stderr_tail.pop_front();
                }
                stderr_tail.push_back(line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }

        let open_timed_out = phase == "opening_device" && phase_started.elapsed() >= open_timeout;
        let progress_timed_out =
            phase != "opening_device" && last_progress.elapsed() >= progress_timeout;
        if open_timed_out || progress_timed_out {
            let _ = files.request_cancel();
            let primary = if open_timed_out {
                format!(
                    "ALLWINNER_FEL_OPEN_TIMEOUT:{}ms:location={usb_location}",
                    open_timeout.as_millis()
                )
            } else {
                format!(
                    "MAINLINE_USB_PROGRESS_TIMEOUT:{}ms:phase={phase}",
                    progress_timeout.as_millis()
                )
            };
            return Err(mainline_error_with_cleanup(
                &primary,
                terminate_mainline_worker(&process_job, &mut child),
            ));
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("MAINLINE_WORKER_WAIT:{error}"))?
        {
            if status.success() {
                update_mainline_task_progress(
                    workbench,
                    task_id,
                    "complete",
                    total_bytes,
                    total_bytes,
                );
                return Ok(());
            }
            let detail = stderr_tail.into_iter().collect::<Vec<_>>().join(" | ");
            return Err(format!(
                "MAINLINE_WORKER_FAILED:exit={}:{}",
                status.code().unwrap_or(-1),
                if detail.is_empty() {
                    "no diagnostic output"
                } else {
                    &detail
                }
            ));
        }
    }
}

struct MainlineAuditOutcome<'a> {
    action: &'a str,
    result: &'a str,
    error: Option<&'a str>,
}

fn publish_mainline_audit(
    app: &AppHandle,
    scope: &ChannelScope,
    session_id: &str,
    artifacts: &[ValidatedMainlineArtifact],
    entry_address: u32,
    outcome: MainlineAuditOutcome<'_>,
) -> Result<(), String> {
    let artifact_metadata = artifacts
        .iter()
        .map(|artifact| {
            json!({
                "role": artifact.role,
                "loadAddress": format!("0x{:08x}", artifact.load_address),
                "sizeBytes": artifact.size_bytes,
                "sha256": artifact.sha256,
            })
        })
        .collect::<Vec<_>>();
    crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "ai".to_owned(),
            actor_id: "linux-mcp-gateway".to_owned(),
            category: "flash".to_owned(),
            action: outcome.action.to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", scope.device_id),
            session_id: Some(session_id.to_owned()),
            session_kind: Some("allwinner_mainline_boot".to_owned()),
            session_title: Some("Allwinner 主线系统 FEL 启动".to_owned()),
            end_session: outcome.action == "mainline_boot_completed",
            correlation_id: Some(session_id.to_owned()),
            causation_id: None,
            result: outcome.result.to_owned(),
            error_code: outcome
                .error
                .map(|item| crate::audit::machine_error_code(item, "MAINLINE_BOOT_ACTION_FAILED")),
            summary: if outcome.action == "mainline_boot_intended" {
                "准备通过固定 Allwinner FEL 单板装载并启动已校验主线镜像".to_owned()
            } else if outcome.result == "success" {
                "主线镜像已写入声明地址并执行声明入口".to_owned()
            } else {
                "主线镜像 FEL 启动未通过确定性执行结果".to_owned()
            },
            metadata: json!({
                "chipVendor": scope.chip_vendor,
                "flasherKind": scope.flasher_kind,
                "usbBinding": scope.usb_binding,
                "entryAddress": format!("0x{entry_address:08x}"),
                "artifacts": artifact_metadata,
            }),
        },
    )?;
    Ok(())
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct FlashOptionsRequest {
    /// Backend-defined flash mode advertised by lynx_flash_capabilities.
    mode: Option<String>,
    /// Enable vendor verification when supported.
    verify: Option<bool>,
    /// Backend-defined post-flash action advertised by lynx_flash_capabilities.
    post_flash_action: Option<String>,
    /// Backend-specific object validated against lynx_flash_capabilities.vendorOptions.
    /// Rockchip raw eMMC: {"raw":true,"loaderPath":"/local/loader.bin","medium":"emmc"}; firmwarePath accepts .img/.img.xz/.img.gz. Requires full_erase and verification.
    vendor_options: Option<Value>,
}

const LARGE_ALLWINNER_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

fn ensure_reliable_flash_link(
    chip_vendor: &str,
    flasher_kind: &str,
    usb_binding: &str,
    usb_speed: &str,
    firmware_bytes: u64,
    allow_full_speed_large_image: bool,
) -> Result<(), String> {
    if chip_vendor == "allwinner"
        && flasher_kind == "openixcli"
        && usb_speed == "full"
        && firmware_bytes > LARGE_ALLWINNER_IMAGE_BYTES
        && !allow_full_speed_large_image
    {
        return Err(format!(
            "USB_LINK_TOO_SLOW:Allwinner FEL/FES target {usb_binding} is connected at Full Speed (12 Mbps), but the firmware is {} MiB. Large-image flashing is blocked because this link can time out after erase or during partition writes. Connect the OTG cable directly to a USB 2.0/3.0 host port without a hub and verify usbSpeed=high (480 Mbps) before retrying.",
            firmware_bytes.div_ceil(1024 * 1024)
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OpenSerialRequest {
    /// Serial device path, for example /dev/ttyACM0 or COM3.
    port: String,
    /// Baud rate such as 115200.
    #[serde(alias = "baudRate")]
    baud_rate: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SerialHandleRequest {
    /// Handle returned by lynx_serial_open.
    handle: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadSerialRequest {
    /// Handle returned by lynx_serial_open.
    handle: String,
    /// Maximum number of bytes to read, capped at 65536.
    #[serde(alias = "maxBytes")]
    max_bytes: Option<usize>,
    /// Read timeout in milliseconds, capped at 5000.
    #[serde(alias = "timeoutMs")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteSerialRequest {
    /// Handle returned by lynx_serial_open.
    handle: String,
    /// UTF-8 text to write to the serial port.
    data: String,
    /// Append a newline after the text.
    #[serde(alias = "appendNewline")]
    append_newline: Option<bool>,
}

const K230_UPGRADE_INTERRUPT: &[u8] = &[0x03];
const K230_UPGRADE_PROMPT: &[u8] = b"msh />";
const K230_UPGRADE_COMMAND: &[u8] = b"reboot_to_upgrade\r\n";
const K230_STALE_DRAIN_PASSES: usize = 4;
const K230_UPGRADE_PROMPT_TIMEOUT: Duration = Duration::from_secs(20);
const K230_UPGRADE_USB_TIMEOUT: Duration = Duration::from_secs(12);

const ALLWINNER_FEL_INTERRUPT: &[u8] = &[0x03];
const ALLWINNER_UBOOT_PROMPT: &[u8] = b"=>";
const ALLWINNER_UBOOT_EFEX_COMMAND: &[u8] = b"efex\r\n";
const ALLWINNER_SERIAL_SHELL_EFEX_COMMAND: &[u8] = b"reboot efex\r\n";
const ALLWINNER_ADB_EFEX_COMMAND: &str = "reboot efex";
const ALLWINNER_UBOOT_PROMPT_TIMEOUT: Duration = Duration::from_secs(20);
const ALLWINNER_SHELL_PROMPT_TIMEOUT: Duration = Duration::from_secs(5);
const ALLWINNER_FEL_USB_TIMEOUT: Duration = Duration::from_secs(30);
const ALLWINNER_STALE_DRAIN_PASSES: usize = 4;
const ALLWINNER_EFEX_TX_SETTLE: Duration = Duration::from_millis(350);

fn contains_allwinner_uboot_prompt(console: &[u8]) -> bool {
    console
        .windows(ALLWINNER_UBOOT_PROMPT.len())
        .enumerate()
        .any(|(index, window)| {
            if window != ALLWINNER_UBOOT_PROMPT {
                return false;
            }
            let starts_line = index == 0 || matches!(console[index - 1], b'\r' | b'\n');
            let end = index + ALLWINNER_UBOOT_PROMPT.len();
            let followed_by_space = end == console.len()
                || console
                    .get(end)
                    .is_some_and(|byte| byte.is_ascii_whitespace());
            starts_line && followed_by_space
        })
}

fn contains_serial_shell_prompt(console: &[u8], prompt: &str) -> bool {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return false;
    }
    let console = String::from_utf8_lossy(console);
    if console.trim_end().ends_with(prompt) {
        return true;
    }

    // USB serial adapters may expose a complete prompt line followed by the
    // beginning of the next prompt in a later packet. Accept a completed
    // shell-shaped line anywhere in the observation window, while keeping a
    // one-character prompt such as `#` from matching arbitrary log lines.
    console.lines().any(|line| {
        let line = line.trim_end();
        line.ends_with(prompt)
            && (prompt.len() > 1 || line == prompt || line.contains('@') || line.contains(":/"))
    })
}

fn validate_allwinner_fel_scope(chip_vendor: &str, flasher_kind: &str) -> Result<(), String> {
    if chip_vendor == "allwinner" && flasher_kind == "openixcli" {
        Ok(())
    } else {
        Err(format!(
            "ALLWINNER_FEL_UNSUPPORTED:{chip_vendor}:{flasher_kind}"
        ))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
// The full `*_efex` wire values are part of the public MCP schema. Keeping the
// suffix on every variant makes accidental protocol renames visible in review.
#[allow(clippy::enum_variant_names)]
enum AllwinnerFelMethod {
    AdbRebootEfex,
    UbootSerialEfex,
    SerialShellRebootEfex,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AllwinnerFelTransitionRequest {
    /// Fixed transition recipe. No caller-provided command text is accepted.
    method: AllwinnerFelMethod,
    /// Handle returned by lynx_serial_open. Required for either serial recipe.
    handle: Option<String>,
    /// Keep the bound serial reader open while the flash worker boots vendor
    /// U-Boot/FES so the caller can collect boot diagnostics. Opt-in only.
    #[serde(default)]
    preserve_serial_monitor: bool,
    /// Pass the same explicit fixed-recipe override as lynx_allwinner_enter_fel.
    #[serde(default)]
    allow_unverified_serial_shell: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AllwinnerFelRequest {
    /// Fixed transition recipe. No caller-provided shell or U-Boot command is accepted.
    method: AllwinnerFelMethod,
    /// Handle returned by lynx_serial_open. Required for either serial recipe.
    handle: Option<String>,
    /// Explicit approval for rebooting or power-cycling the selected board.
    confirm_destructive: bool,
    /// Keep the serial reader available for diagnostics after sending efex.
    #[serde(default)]
    preserve_serial_monitor: bool,
    /// Allow the fixed `reboot efex` recipe to be sent when the selected
    /// serial console is connected but an otherwise booted image does not
    /// render its configured shell prompt. No caller-provided command text is
    /// accepted, and the FEL USB device must still appear before success.
    #[serde(default)]
    allow_unverified_serial_shell: bool,
}

struct AllwinnerFelOutcome {
    response: String,
    online: bool,
    hardware_guard: crate::hardware_safety::HardwareActivityGuard,
}

fn validate_k230_upgrade_scope(chip_vendor: &str, flasher_kind: &str) -> Result<(), String> {
    if chip_vendor == "kendryte" && flasher_kind == "k230" {
        Ok(())
    } else {
        Err(format!(
            "K230_UPGRADE_MODE_UNSUPPORTED:{chip_vendor}:{flasher_kind}"
        ))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AdbShellRequest {
    /// Optional ADB serial. When omitted, the selected/only device is used.
    serial: Option<String>,
    /// Shell command to execute on the target.
    command: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum McpPowerAction {
    On,
    Off,
    Cycle,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PowerControlRequest {
    /// Requested relay action. Cycle turns power off, waits, then turns it on.
    action: McpPowerAction,
    /// Optional off interval for cycle, clamped to 100..30000 ms.
    off_time_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SupervisorAutomaticTaskRequest {
    /// Saved visual device-flow task ID from the AI Supervisor page.
    task_id: i64,
    /// Explicit operator approval for high-risk nodes. Defaults to false.
    confirm_high_risk: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CameraCaptureRequest {
    /// Maximum returned width, clamped to 320..1920 pixels.
    max_width: Option<u32>,
    /// JPEG quality, clamped to 50..90.
    quality: Option<u8>,
}

fn capture_camera_jpeg(
    binding: &str,
    max_width: u32,
    quality: u8,
) -> Result<(Vec<u8>, u32, u32, String), String> {
    use nokhwa::{
        pixel_format::RgbFormat,
        utils::{CameraFormat, RequestedFormat, RequestedFormatType},
        Camera,
    };

    let backend =
        nokhwa::native_api_backend().ok_or_else(|| "CAMERA_BACKEND_UNAVAILABLE".to_owned())?;
    let cameras = std::panic::catch_unwind(|| nokhwa::query(backend))
        .map_err(|_| "CAMERA_ENUMERATION_FAILED".to_owned())?
        .map_err(|error| format!("CAMERA_ENUMERATION_FAILED:{error}"))?;
    let info = cameras
        .into_iter()
        .find(|camera| crate::workbench::commands::camera_binding_key(camera) == binding)
        .ok_or_else(|| "CAMERA_OFFLINE:the board-bound USB camera is not available".to_owned())?;
    let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
        CameraFormat::new_from(1280, 720, nokhwa::utils::FrameFormat::MJPEG, 30),
    ));
    let mut camera = Camera::new(info.index().clone(), requested)
        .map_err(|error| format!("CAMERA_OPEN_FAILED:{error}"))?;
    camera
        .open_stream()
        .map_err(|error| format!("CAMERA_OPEN_FAILED:{error}"))?;
    let frame = camera
        .frame()
        .map_err(|error| format!("CAMERA_CAPTURE_FAILED:{error}"))?;
    let decoded = frame
        .decode_image::<RgbFormat>()
        .map_err(|error| format!("CAMERA_DECODE_FAILED:{error}"))?;
    let (source_width, source_height) = decoded.dimensions();
    let requested_width = max_width.clamp(320, 1920).min(source_width);
    let requested_height = ((u64::from(source_height) * u64::from(requested_width))
        / u64::from(source_width.max(1)))
    .clamp(1, 1080) as u32;
    let image = if requested_width != source_width || requested_height != source_height {
        image::imageops::resize(
            &decoded,
            requested_width,
            requested_height,
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let (width, height) = image.dimensions();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(50, 90))
        .encode_image(&image)
        .map_err(|error| format!("CAMERA_ENCODE_FAILED:{error}"))?;
    if jpeg.len() > MAX_CAMERA_JPEG_BYTES {
        return Err("CAMERA_IMAGE_TOO_LARGE:captured JPEG exceeds 5 MiB".to_owned());
    }
    Ok((jpeg, width, height, info.human_name()))
}

#[tool_router]
impl LynxMcp {
    #[tool(
        name = "lynx_workbench_status",
        description = "Discover this channel's LYNX hardware capabilities, exact tool workflow, board scope, and supported task telemetry before operating hardware."
    )]
    async fn workbench_status(&self) -> String {
        self.record_interaction("lynx_workbench_status", "连接", "AI 检查 LYNX 工作台状态");
        let scope = self.live_channel_scope();
        let vendor_policy = scope
            .as_ref()
            .and_then(|scope| crate::workbench::flasher::flash_safety_policy(&scope.chip_vendor));
        let hardware_activities = self
            .app_handle
            .as_ref()
            .map(crate::hardware_safety::active_reasons)
            .unwrap_or_default();
        json!({
            "online": true,
            "product": "LYNX",
            "apiVersion": "0.2",
            "appVersion": env!("CARGO_PKG_VERSION"),
            "gateway": "mcp",
            "transport": "streamable-http",
            "flashToolsEnabled": self.app_handle.is_some(),
            "downloadToolsEnabled": self.app_handle.is_some(),
            "adbToolsEnabled": self.app_handle.is_some(),
            "serialToolsEnabled": true,
            "powerToolsEnabled": self.app_handle.is_some(),
            "hardwareSafety": {
                "busy": !hardware_activities.is_empty(),
                "activities": hardware_activities,
            },
            "projectId": scope.as_ref().map(|scope| scope.project_id),
            "projectName": scope.as_ref().map(|scope| scope.project_name.as_str()),
            "mcpName": scope.as_ref().map(|scope| scope.mcp_server_name.as_str()),
            "deviceId": scope.as_ref().map(|scope| scope.device_id),
            "chipVendor": scope.as_ref().map(|scope| scope.chip_vendor.as_str()),
            "flasherKind": scope.as_ref().map(|scope| scope.flasher_kind.as_str()),
            "serialPort": scope.as_ref().map(|scope| scope.serial_port.as_str()),
            "usbBinding": scope.as_ref().map(|scope| scope.usb_binding.as_str()),
            "adbSerial": scope.as_ref().map(|scope| scope.adb_serial.as_str()),
            "cameraConfigured": scope.as_ref().is_some_and(|scope| !scope.camera_binding.is_empty()),
            "configurationMode": "live",
            "flashCapabilitiesTool": "lynx_flash_capabilities",
            "capabilityManifest": {
                "firmware": ["lynx_inspect_firmware", "lynx_download_firmware"],
                "flash": ["lynx_flash_capabilities", "lynx_scan_flash_devices", "lynx_start_flash", "lynx_allwinner_boot_mainline"],
                "tasks": ["lynx_workbench_list_tasks", "lynx_workbench_task_status", "lynx_workbench_cancel_task"],
                "supervisor": ["lynx_supervisor_list_automatic_tasks", "lynx_supervisor_trigger_automatic_task"],
                "power": ["lynx_power_capabilities", "lynx_power_control", "lynx_power_status"],
                "serial": ["lynx_serial_list", "lynx_serial_open", "lynx_serial_read", "lynx_serial_write", "lynx_allwinner_enter_fel", "lynx_rockchip_enter_loader", "lynx_k230_enter_upgrade_mode", "lynx_serial_close"],
                "adb": ["lynx_adb_list", "lynx_adb_shell", "lynx_allwinner_enter_fel", "lynx_rockchip_enter_loader"],
                "skills": ["lynx_skills_search", "lynx_skills_get", "lynx_skills_read", "lynx_skills_list_systems"],
                "camera": ["lynx_camera_capture"]
            },
            "flashStatusFields": ["state", "phase", "progressPct", "currentPartition", "partitionProgressPct", "transferredBytes", "queuedBytes", "committedBytes", "bytesPerSecond", "usbSpeed", "verifyState", "verifyMethod", "loaderCapabilities", "verifyErrorCode", "postFlashState", "error"],
            "recommendedFlashWorkflow": [
                "lynx_workbench_status",
                "lynx_flash_capabilities",
                "lynx_scan_flash_devices",
                "lynx_inspect_firmware (when the image format is supported)",
                "lynx_download_firmware (when the artifact is remote), then poll lynx_workbench_task_status",
                "lynx_start_flash with confirmDestructive=true after target and firmware verification; follow vendorPolicy.eraseStrategy",
                "for Allwinner, loaderPath is always RAM-only; NAND/SPI NAND use a component package with mode=full_erase for initial layout creation (or partition_erase for a bounded replacement), while SPI NOR, SD/SD NAND, eMMC and UFS may use raw=true with mode=command after the runtime storage preflight",
                "for Rockchip in normal mode, use rockchipEnterLoader in lynx_start_flash, or lynx_rockchip_enter_loader separately; only confirmed Loader permits continuation, timeout requires physical Maskrom intervention",
                "poll lynx_workbench_task_status until success, failed, or cancelled"
            ],
            "vendorPolicy": vendor_policy,
            "bootModePolicy": "Generic ADB/Shell remains prohibited from entering programming modes. Dedicated board-scoped exceptions exist: lynx_rockchip_enter_loader sends fixed reboot loader and confirms Loader (not Maskrom) at the saved USB position; lynx_allwinner_enter_fel uses either the compiled-in exact `reboot efex` command or a power-only cycle followed by Ctrl+C and exact `efex` at the bound U-Boot serial prompt; lynx_k230_enter_upgrade_mode uses its vendor-supported serial sequence. The Allwinner recipe never models the physical FEL button as a relay and never requires three GPIOs. syscon unbind, reboot-mode writes, devmem, and caller-provided transition commands remain prohibited.",
            "nonBlockingObservations": [
                "board driver warnings",
                "transient network route failures",
                "NAND bad-block counts handled by UBI"
            ]
        })
        .to_string()
    }

    #[tool(
        name = "lynx_skills_search",
        description = "Search vetted read-only Linux/SoC skills for this SDK channel by query and optional system/category filters. Skills provide guidance only and never execute actions."
    )]
    async fn skills_search(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillSearchRequest>,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope();
        let vendor = scope.as_ref().map(|scope| scope.chip_vendor.as_str());
        let result = crate::skills::search(SkillDomain::Linux, vendor, request)?;
        self.record_interaction("lynx_skills_search", "Skills", "AI 检索 Linux SDK 技能");
        Ok(result)
    }

    #[tool(
        name = "lynx_skills_get",
        description = "Get metadata, hashes, references and required existing tools for one exact Linux/SoC skill."
    )]
    async fn skills_get(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillGetRequest>,
    ) -> Result<String, String> {
        let result = crate::skills::get(SkillDomain::Linux, request)?;
        self.record_interaction("lynx_skills_get", "Skills", "AI 读取 Linux SDK 技能元数据");
        Ok(result)
    }

    #[tool(
        name = "lynx_skills_read",
        description = "Read a bounded UTF-8 chunk from SKILL.md or a vetted relative reference file. Absolute paths, traversal, scripts and binaries are prohibited."
    )]
    async fn skills_read(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillReadRequest>,
    ) -> Result<String, String> {
        let result = crate::skills::read(SkillDomain::Linux, request)?;
        self.record_interaction("lynx_skills_read", "Skills", "AI 读取 Linux SDK 技能正文");
        Ok(result)
    }

    #[tool(
        name = "lynx_skills_list_systems",
        description = "List normalized system filters available in the Linux/SoC built-in skills catalog."
    )]
    async fn skills_list_systems(&self) -> Result<String, String> {
        self.record_interaction(
            "lynx_skills_list_systems",
            "Skills",
            "AI 查询 Linux 技能系统",
        );
        crate::skills::list_systems(SkillDomain::Linux)
    }

    #[tool(
        name = "lynx_camera_capture",
        description = "Capture one JPEG image from the USB camera fixed to this Linux board. The caller cannot select a device or output path."
    )]
    async fn camera_capture(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<CameraCaptureRequest>,
    ) -> Result<CallToolResult, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_camera_capture requires a saved Linux board channel".to_owned()
        })?;
        if scope.camera_binding.trim().is_empty() {
            return Err(
                "CAMERA_NOT_CONFIGURED:select a USB camera in Linux board settings".to_owned(),
            );
        }
        let app = self.app_handle.as_ref().ok_or_else(|| {
            "CAMERA_HOST_REQUIRED:camera capture requires desktop LYNX".to_owned()
        })?;
        let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
        let guard = safety.try_begin(
            "USB 摄像头拍照",
            "Linux 开发板 MCP 单帧采集",
            format!("linux-board:{}", scope.device_id),
        )?;
        let binding = scope.camera_binding.clone();
        let max_width = request.max_width.unwrap_or(1280);
        let quality = request.quality.unwrap_or(80);
        let capture = tauri::async_runtime::spawn_blocking(move || {
            let _guard = guard;
            capture_camera_jpeg(&binding, max_width, quality)
        });
        let (jpeg, width, height, camera_name) =
            tokio::time::timeout(Duration::from_secs(10), capture)
                .await
                .map_err(|_| {
                    "CAMERA_CAPTURE_TIMEOUT:camera did not return a frame within 10 seconds"
                        .to_owned()
                })?
                .map_err(|error| format!("CAMERA_CAPTURE_FAILED:{error}"))??;
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        self.record_interaction(
            "lynx_camera_capture",
            "摄像头",
            "AI 拍摄开发板 USB 摄像头单帧",
        );
        Ok(CallToolResult::success(vec![
            Content::text(
                json!({"width": width, "height": height, "bytes": jpeg.len(), "camera": camera_name})
                    .to_string(),
            ),
            Content::image(encoded, "image/jpeg"),
        ]))
    }

    #[tool(
        name = "lynx_power_capabilities",
        description = "Describe this board channel's configured relay adapter, supported actions, GPIO binding, polarity, and state-read limitations."
    )]
    async fn power_capabilities(&self) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_power_capabilities requires a board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Power tools require the desktop-hosted LYNX Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let device = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::device_by_id(&db, scope.device_id)
                .map_err(|error| error.to_string())?
        };
        let profile =
            serde_json::from_str::<Value>(&device.power_profile_json).unwrap_or(Value::Null);
        let is_lynx_power = profile.get("type").and_then(Value::as_str) == Some("ch347_gpio");
        let adapter = if is_lynx_power {
            "lynx_power"
        } else if device.power_profile_json.trim().is_empty() {
            "none"
        } else {
            "ascii_cdc"
        };
        let configured = adapter != "none";
        let actions = if configured {
            vec!["on", "off", "cycle"]
        } else {
            Vec::new()
        };
        let public_profile = if is_lynx_power {
            json!({
                "controllerIndex": profile.get("deviceIndex").cloned().unwrap_or(Value::from(0)),
                "controllerVid": format!("{:04x}", device.power_controller_vid),
                "controllerPid": format!("{:04x}", device.power_controller_pid),
                "controllerSerial": device.power_controller_serial,
                "controllerInterface": device.power_controller_interface_number,
                "channel": profile.get("gpio").cloned().unwrap_or(Value::from(0)),
                "secondaryPowerChannel": if profile.get("roleMode").and_then(Value::as_str) == Some("dualPower") {
                    Value::String(device.boot_channel.clone())
                } else {
                    Value::Null
                },
                "activeHigh": profile.get("activeHigh").cloned().unwrap_or(Value::from(true)),
                "roleMode": profile.get("roleMode").cloned().unwrap_or(Value::from("independent")),
            })
        } else {
            profile
        };
        self.record_interaction("lynx_power_capabilities", "电源", "AI 查询 Lynx Power 能力");
        Ok(json!({
            "deviceId": scope.device_id,
            "configured": configured,
            "adapter": adapter,
            "actions": actions,
            "unavailableReason": if configured { Value::Null } else { Value::String("POWER_PROFILE_MISSING".to_owned()) },
            "powerChannel": device.power_channel,
            "powerOffWaitMs": device.power_off_wait_ms,
            "profile": public_profile,
            "stateRead": if is_lynx_power { "last_known (the controller channel is write-only)" } else { "hardware profile dependent" },
            "platformAvailable": true,
        }).to_string())
    }

    #[tool(
        name = "lynx_power_control",
        description = "Control the channel-scoped board relay through its configured adapter. Supports on, off, and cycle; cycle uses the saved off delay unless offTimeMs is supplied."
    )]
    async fn power_control(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<PowerControlRequest>,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_power_control requires a board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Power tools require the desktop-hosted LYNX Gateway.".to_owned())?;
        crate::hardware_safety::ensure_idle(app_handle, "Linux MCP 电源操作")?;
        let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard = safety.try_begin(
            "Linux MCP",
            "电源操作",
            format!("device:{}", scope.device_id),
        )?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let device = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::device_by_id(&db, scope.device_id)
                .map_err(|error| error.to_string())?
        };
        let lock = workbench.device_lock(scope.device_id)?;
        let _guard = lock
            .try_lock()
            .map_err(|_| "DEVICE_BUSY:another device operation is running".to_owned())?;
        let (action_name, response) = match request.action {
            McpPowerAction::On => (
                "on",
                crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOn,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?,
            ),
            McpPowerAction::Off => (
                "off",
                crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOff,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?,
            ),
            McpPowerAction::Cycle => {
                crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOff,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?;
                let off_time_ms = request
                    .off_time_ms
                    .unwrap_or(device.power_off_wait_ms)
                    .clamp(100, 30_000);
                std::thread::sleep(Duration::from_millis(off_time_ms));
                let response = crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOn,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?;
                ("cycle", response)
            }
        };
        self.record_interaction(
            "lynx_power_control",
            "电源",
            format!("AI 执行继电器电源 {action_name}"),
        );
        Ok(json!({
            "deviceId": scope.device_id,
            "action": action_name,
            "state": if action_name == "off" { "off" } else { "on" },
            "response": response,
        })
        .to_string())
    }

    #[tool(
        name = "lynx_power_status",
        description = "Return Lynx Power state for the scoped board. Write-only controllers report the latest successful commanded state."
    )]
    async fn power_status(&self) -> Result<String, String> {
        let scope = self
            .live_channel_scope()
            .ok_or_else(|| "CHANNEL_SCOPE:lynx_power_status requires a board channel".to_owned())?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Power tools require the desktop-hosted LYNX Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let state = crate::workbench::power::last_known_power_state(&workbench, scope.device_id)?;
        self.record_interaction("lynx_power_status", "电源", "AI 查询继电器电源状态");
        Ok(json!({
            "deviceId": scope.device_id,
            "state": state,
            "source": "last_known_command",
        })
        .to_string())
    }

    #[tool(
        name = "lynx_adb_list",
        description = "Return the board-scoped ADB binding, authoritative online state, and matching device details."
    )]
    async fn adb_list(&self) -> Result<String, String> {
        self.record_interaction("lynx_adb_list", "ADB", "AI 查询 ADB 设备");
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "ADB tools require the desktop-hosted Gateway.".to_string())?;
        let mut devices = crate::adb::commands::adb_list_devices(
            app_handle.clone(),
            app_handle.state::<crate::adb::commands::AdbState>(),
        )
        .await
        .map_err(|error| error.message)?;
        let scope = self.live_channel_scope();
        if let Some(scope) = &scope {
            devices.retain(|device| device.serial == scope.adb_serial);
        }
        let online = devices.iter().any(|device| device.state == "device");
        serde_json::to_string_pretty(&json!({
            "configuredSerial": scope.as_ref().map(|scope| scope.adb_serial.as_str()),
            "online": online,
            "state": if online { "online" } else if devices.is_empty() { "not_found" } else { "not_ready" },
            "devices": devices,
        }))
        .map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_adb_shell",
        description = "Execute one shell command through ADB on a configured device."
    )]
    async fn adb_shell(
        &self,
        rmcp::handler::server::wrapper::Parameters(mut request): rmcp::handler::server::wrapper::Parameters<
            AdbShellRequest,
        >,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope();
        if let Some(scope) = &scope {
            if scope.adb_serial.is_empty() {
                return Err(format!("DEVICE_SCOPE:单板 {} 未绑定 ADB", scope.device_id));
            }
            if request
                .serial
                .as_deref()
                .is_some_and(|serial| serial != scope.adb_serial)
            {
                return Err(format!(
                    "DEVICE_SCOPE:该 AI 通道只允许访问 ADB {}",
                    scope.adb_serial
                ));
            }
            request.serial = Some(scope.adb_serial.clone());
        }
        let adb_device_key = request
            .serial
            .as_deref()
            .map(|serial| format!("adb:{serial}"))
            .unwrap_or_default();
        self.record_device_interaction(
            "lynx_adb_shell",
            "ADB",
            format!("AI 执行 ADB 命令：{} 字节", request.command.len()),
            adb_device_key,
        );
        if request.command.trim().is_empty() {
            return Err("ADB shell command cannot be empty".to_string());
        }
        validate_ai_shell_command(
            &request.command,
            scope.as_ref().map(|scope| scope.chip_vendor.as_str()),
        )?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "ADB tools require the desktop-hosted Gateway.".to_string())?;
        let normalized = request.command.trim().to_ascii_lowercase();
        let _hardware_guard = if normalized == "reboot" || normalized.starts_with("reboot ") {
            crate::hardware_safety::ensure_idle(app_handle, "Linux MCP ADB 重启")?;
            Some(
                app_handle
                    .state::<crate::hardware_safety::HardwareSafetyState>()
                    .try_begin(
                        "Linux MCP",
                        "ADB 重启",
                        request.serial.clone().unwrap_or_default(),
                    )?,
            )
        } else {
            None
        };
        crate::adb::commands::adb_shell_command_core(
            app_handle,
            &app_handle.state::<crate::adb::commands::AdbState>(),
            request.serial,
            request.command,
            crate::adb::commands::AdbOperationActor::Gateway,
        )
        .map_err(|error| error.message)
    }

    #[tool(
        name = "lynx_scan_flash_devices",
        description = "Scan the board-scoped vendor flasher for connected devices. Supports Allwinner FEL/FES, Rockchip Maskrom/Loader, NXP i.MX SDP/Fastboot, Canaan Kendryte K230 BROM/U-Boot, and STM32 USB DFU/DfuSe."
    )]
    async fn scan_flash_devices(&self) -> Result<String, String> {
        self.record_interaction("lynx_scan_flash_devices", "烧录接口", "AI 扫描可烧录设备");
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_scan_flash_devices requires a board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .clone()
            .ok_or_else(|| "Vendor device scan requires the desktop-hosted Gateway.".to_owned())?;
        let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard = safety.try_begin_scoped(
            "Linux MCP",
            "USB 烧录设备扫描",
            format!("device:{}", scope.device_id),
            crate::workbench::flasher::flasher_resource_key(
                &scope.flasher_kind,
                &scope.usb_binding,
            ),
        )?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let device_lock = workbench.device_lock(scope.device_id)?;
        let kind = scope.flasher_kind.clone();
        let path = scope.flasher_path.clone();
        let binding = scope.usb_binding.clone();
        let flasher_lock = workbench.flasher_target_lock(&kind, &binding)?;
        let mut devices = tauri::async_runtime::spawn_blocking(move || {
            let _device_guard = device_lock.try_lock().map_err(|_| {
                "DEVICE_BUSY:cannot scan the board's flash interface during a hardware operation"
                    .to_owned()
            })?;
            let _flasher_guard = flasher_lock.try_lock().map_err(|_| {
                format!("FLASHER_BUSY:{kind} is currently performing a hardware operation")
            })?;
            crate::workbench::flasher::scan_flasher_devices(&app_handle, &kind, &path)
        })
        .await
        .map_err(|error| error.to_string())??;
        devices.retain(|device| device.key == binding);
        let online = !devices.is_empty();
        let devices: Vec<Value> = devices
            .into_iter()
            .map(|device| {
                let key = device.key.clone();
                let mut value = serde_json::to_value(device).unwrap_or_else(|_| json!({}));
                let (speed, theoretical_mbps) = crate::workbench::flasher::usb_diagnostics(&key);
                if let Some(object) = value.as_object_mut() {
                    object.insert("usbSpeed".to_owned(), Value::String(speed));
                    object.insert(
                        "usbTheoreticalMbps".to_owned(),
                        theoretical_mbps.map_or(Value::Null, Value::from),
                    );
                }
                value
            })
            .collect();
        let platform = crate::workbench::types::platform_spec(&scope.chip_vendor);
        serde_json::to_string_pretty(&json!({
            "chipVendor": scope.chip_vendor,
            "protocol": platform.map(|item| item.protocol.as_str()),
            "flasherKind": scope.flasher_kind,
            "configuredBinding": scope.usb_binding,
            "online": online,
            "state": if scope.usb_binding.is_empty() { "unbound" } else if online { "online" } else { "not_found" },
            "devices": devices,
        }))
        .map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_inspect_firmware",
        description = "Inspect a firmware image with the current board channel's vendor adapter. This is read-only."
    )]
    async fn inspect_firmware(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            InspectFirmwareRequest,
        >,
    ) -> Result<String, String> {
        self.record_interaction(
            "lynx_inspect_firmware",
            "固件",
            format!("AI 检查固件：{}", request.image_path),
        );
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_inspect_firmware requires a board channel".to_owned()
        })?;
        if scope.chip_vendor != "allwinner" {
            return Err(format!(
                "INSPECT_UNSUPPORTED:{} does not expose a firmware inspector",
                scope.chip_vendor
            ));
        }
        let result = crate::firmware::commands::firmware_parse_image(request.image_path)?;
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_download_firmware",
        description = "Download a firmware artifact selected by the remote AI from the saved SDK channel's Linux host into LYNX's automatic per-channel firmware cache through SFTP. Pass remotePath as the exact absolute Linux file path. LYNX always verifies SHA-256, reuses a matching cached file, and writes new content through a .part file before atomic rename."
    )]
    async fn download_firmware(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            DownloadFirmwareRequest,
        >,
    ) -> Result<String, String> {
        let app_handle = self
            .app_handle
            .clone()
            .ok_or_else(|| "SFTP download requires the desktop-hosted Gateway.".to_owned())?;
        let live_scope = self.live_channel_scope();
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let project = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            let projects =
                crate::workbench::storage::list_projects(&db).map_err(|error| error.to_string())?;
            if let Some(project_id) = request.project_id {
                projects
                    .into_iter()
                    .find(|project| project.id == Some(project_id))
                    .ok_or_else(|| format!("DOWNLOAD_PROJECT:SDK 通道 {project_id} 不存在"))?
            } else if let Some(scope) = &live_scope {
                projects
                    .into_iter()
                    .find(|project| project.id == Some(scope.project_id))
                    .ok_or_else(|| {
                        format!("DOWNLOAD_PROJECT:SDK 通道 {} 不存在", scope.project_id)
                    })?
            } else {
                return Err("DOWNLOAD_PROJECT:非设备专属 Gateway 必须明确指定 projectId".to_owned());
            }
        };
        if let Some(scope) = &live_scope {
            if project.id != Some(scope.project_id) || project.device_id != Some(scope.device_id) {
                return Err(format!(
                    "CHANNEL_SCOPE:该 MCP 只允许使用 SDK 通道 {}（{}）",
                    scope.project_id, scope.project_name
                ));
            }
        }
        let project_id = project
            .id
            .ok_or_else(|| "DOWNLOAD_PROJECT:SDK 通道缺少持久化 ID".to_owned())?;
        if !request.remote_path.starts_with('/')
            || request.remote_path.contains('\0')
            || request.remote_path.contains(['*', '?', '['])
        {
            return Err(
                "VALIDATION:remotePath must be an exact absolute Linux file path".to_owned(),
            );
        }
        let filename = Path::new(&request.remote_path)
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "VALIDATION:remotePath must include a file name".to_owned())?;
        let cache_dir = app_handle
            .path()
            .app_cache_dir()
            .map_err(|error| format!("LOCAL_CACHE:{error}"))?
            .join("firmware")
            .join(format!("project-{project_id}"));
        let local_path = cache_dir.join(filename);
        self.record_interaction(
            "lynx_download_firmware",
            "固件传输",
            format!("AI 请求下载固件：路径 {} 字节", request.remote_path.len()),
        );
        let task_id = crate::workbench::commands::start_download_firmware(
            project_id,
            Some(request.remote_path.clone()),
            Some(cache_dir.to_string_lossy().into_owned()),
            crate::workbench::types::SshCredentials::default(),
            app_handle.clone(),
            workbench.inner(),
            crate::workbench::commands::RemoteTaskActor::Gateway,
        )?;
        Ok(json!({
            "taskId": task_id,
            "projectId": project_id,
            "status": "started",
            "remotePath": request.remote_path,
            "localPath": local_path,
            "verifySha256": true,
            "taskLifetime": "LYNX application process; independent of the MCP session that started it",
            "resumable": true,
            "next": "Poll lynx_workbench_task_status with this taskId from this or any later MCP session until state is success, failed, or cancelled. If LYNX itself restarts, call lynx_download_firmware again to resume its .part file."
        })
        .to_string())
    }

    #[tool(
        name = "lynx_workbench_list_tasks",
        description = "Read the latest 20 persisted tasks and their original output tails for this exact saved SDK/board channel, without opening any hardware. Use this to find a failed flash taskId."
    )]
    async fn workbench_list_tasks(&self) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_workbench_list_tasks requires a saved board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Workbench tasks require the desktop-hosted Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let tasks = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::list_channel_tasks(&db, scope.project_id, scope.device_id)
                .map_err(|error| error.to_string())?
        };
        for task in &tasks {
            self.ensure_task_scope(task)?;
        }
        self.record_interaction(
            "lynx_workbench_list_tasks",
            "任务",
            "AI 读取当前通道最近任务与诊断",
        );
        serde_json::to_string(&json!({
            "projectId": scope.project_id,
            "deviceId": scope.device_id,
            "limit": 20,
            "tasks": tasks.iter().map(task_status_value).collect::<Vec<_>>(),
        }))
        .map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_workbench_task_status",
        description = "Read the current status of an asynchronous LYNX workbench task such as an SFTP firmware download."
    )]
    async fn workbench_task_status(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            WorkbenchTaskRequest,
        >,
    ) -> Result<String, String> {
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Workbench tasks require the desktop-hosted Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let task = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::task_by_id(&db, &request.task_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("TASK_NOT_FOUND:{}", request.task_id))?
        };
        self.ensure_task_scope(&task)?;
        self.record_interaction(
            "lynx_workbench_task_status",
            "任务",
            format!("AI 查询工作台任务 {}", request.task_id),
        );
        serde_json::to_string_pretty(&task_status_value(&task)).map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_supervisor_list_automatic_tasks",
        description = "List visual AI Supervisor device-flow tasks bound to this exact board channel. Secret values are never returned."
    )]
    async fn supervisor_list_automatic_tasks(&self) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_supervisor_list_automatic_tasks requires a board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "AI Supervisor tools require the desktop-hosted Gateway.".to_owned())?;
        let board_id = format!("linux:{}", scope.device_id);
        let tasks = crate::supervisor::channel_automatic_tasks(app_handle, &board_id)?;
        self.record_interaction(
            "lynx_supervisor_list_automatic_tasks",
            "AI 监督",
            format!("AI 查询单板 {} 的自动任务", scope.device_id),
        );
        serde_json::to_string_pretty(&json!({
            "boardId": board_id,
            "tasks": tasks,
        }))
        .map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_supervisor_trigger_automatic_task",
        description = "Trigger one saved visual device-flow task on this exact board. High-risk nodes remain blocked unless confirmHighRisk is explicitly true; legacy workflows are not accepted by this MCP entry point."
    )]
    async fn supervisor_trigger_automatic_task(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            SupervisorAutomaticTaskRequest,
        >,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_supervisor_trigger_automatic_task requires a board channel"
                .to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "AI Supervisor tools require the desktop-hosted Gateway.".to_owned())?;
        if request.task_id <= 0 {
            return Err("SUPERVISOR_AUTOMATIC_TASK_ID_INVALID".to_owned());
        }
        let board_id = format!("linux:{}", scope.device_id);
        let approved = request.confirm_high_risk.unwrap_or(false);
        let started_task_id = crate::supervisor::trigger_channel_automatic_task(
            app_handle,
            &board_id,
            request.task_id,
            approved,
        )?;
        self.record_interaction(
            "lynx_supervisor_trigger_automatic_task",
            "AI 监督",
            format!(
                "AI 触发单板 {} 的自动任务 {}（高风险确认={}）",
                scope.device_id, request.task_id, approved
            ),
        );
        Ok(json!({
            "automaticTaskId": request.task_id,
            "taskId": started_task_id,
            "boardId": board_id,
            "state": "running",
            "highRiskApproved": approved,
            "next": "Poll lynx_workbench_task_status with taskId until terminal, then inspect the AI Supervisor run and audit pages.",
        })
        .to_string())
    }

    #[tool(
        name = "lynx_workbench_cancel_task",
        description = "Cooperatively cancel a running LYNX workbench task. Terminal tasks are returned unchanged."
    )]
    async fn workbench_cancel_task(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            WorkbenchTaskRequest,
        >,
    ) -> Result<String, String> {
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Workbench tasks require the desktop-hosted Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let task = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::task_by_id(&db, &request.task_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("TASK_NOT_FOUND:{}", request.task_id))?
        };
        self.ensure_task_scope(&task)?;
        let cancellation_requested = if task.status == "running" {
            let tasks = workbench
                .cancellations
                .lock()
                .map_err(|error| error.to_string())?;
            let cancellation = tasks
                .get(&request.task_id)
                .ok_or_else(|| format!("TASK_NOT_RUNNING:{}", request.task_id))?;
            cancellation.store(true, Ordering::Relaxed);
            true
        } else {
            false
        };
        self.record_interaction(
            "lynx_workbench_cancel_task",
            "任务",
            format!("AI 请求取消工作台任务 {}", request.task_id),
        );
        Ok(json!({
            "taskId": request.task_id,
            "cancellationRequested": cancellation_requested,
            "state": task.status,
        })
        .to_string())
    }

    #[tool(
        name = "lynx_flash_capabilities",
        description = "Describe the chip vendor, flasher backend, target binding, and validated flash options for this LYNX board channel."
    )]
    async fn flash_capabilities(&self) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_flash_capabilities requires a board channel".to_owned()
        })?;
        let app_handle = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "Flash capabilities require the desktop-hosted Gateway.".to_owned())?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let device = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::device_by_id(&db, scope.device_id)
                .map_err(|error| error.to_string())?
        };
        let capabilities = crate::workbench::flasher::flash_capabilities(&device)?;
        Ok(json!({
            "deviceId": scope.device_id,
            "target": scope.usb_binding,
            "capabilities": capabilities,
        })
        .to_string())
    }

    #[tool(
        name = "lynx_start_flash",
        description = "DESTRUCTIVE: flash this channel's bound board through its configured vendor adapter. Allwinner loaderPath is RAM-only: use a component package with mode=full_erase for initial NAND/SPI NAND layout creation, or raw=true and mode=command for SPI NOR, SD/SD NAND, eMMC and UFS. Query lynx_flash_capabilities first."
    )]
    async fn start_flash(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            StartFlashRequest,
        >,
    ) -> Result<String, String> {
        let scope = self
            .live_channel_scope()
            .ok_or_else(|| "CHANNEL_SCOPE:lynx_start_flash requires a board channel".to_owned())?;
        self.record_device_interaction(
            "lynx_start_flash",
            "烧录",
            format!(
                "AI 请求通过 {} 烧录固件 {}",
                scope.flasher_kind, request.firmware_path
            ),
            format!("usb:{}", scope.usb_binding),
        );
        let app_handle = self.app_handle.clone().ok_or_else(|| {
            "Firmware flashing requires the desktop-hosted LYNX Gateway.".to_string()
        })?;
        if !request.confirm_destructive {
            return Err(
                "Destructive operation rejected: confirmDestructive must be true after the AI verifies the channel-scoped target and firmware."
                    .to_owned(),
            );
        }
        if !Path::new(&request.firmware_path).is_file() {
            return Err(format!("FIRMWARE_NOT_FOUND:{}", request.firmware_path));
        }
        if request.allwinner_enter_fel.is_some() && request.rockchip_enter_loader.is_some() {
            return Err(
                "FLASH_TRANSITION_CONFLICT:select only this board's transition recipe".to_owned(),
            );
        }
        let firmware_bytes = std::fs::metadata(&request.firmware_path)
            .map_err(|error| format!("FIRMWARE_METADATA:{}:{error}", request.firmware_path))?
            .len();
        let (usb_speed, _) = crate::workbench::flasher::usb_diagnostics(&scope.usb_binding);
        ensure_reliable_flash_link(
            &scope.chip_vendor,
            &scope.flasher_kind,
            &scope.usb_binding,
            &usb_speed,
            firmware_bytes,
            request.allow_full_speed_large_image,
        )?;
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let workbench_state = workbench.inner().clone();
        let device = {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            crate::workbench::storage::device_by_id(&db, scope.device_id)
                .map_err(|error| error.to_string())?
        };
        let options = request.options.unwrap_or_default();
        let override_args = crate::workbench::flasher::adapter_override_args(
            &device,
            crate::workbench::flasher::AdapterFlashOptions {
                mode: options.mode,
                verify: options.verify,
                post_flash_action: options.post_flash_action,
                vendor_options: options.vendor_options,
            },
        )?;
        if override_args
            .as_ref()
            .is_some_and(|args| args.iter().any(|arg| arg == "--raw"))
            && scope.chip_vendor == "rockchip"
            && request.rockchip_enter_loader.is_some()
        {
            return Err("ROCKCHIP_RAW_TRANSITION:raw preparation must precede hardware actions; enter the bound board's Maskrom/Loader before submitting raw flashing".to_owned());
        }
        let mut loader_transition = None;
        let mut preserve_serial_monitor = false;
        let hardware_guard = if let Some(transition) = request.allwinner_enter_fel {
            preserve_serial_monitor = transition.preserve_serial_monitor;
            let outcome = self
                .execute_allwinner_enter_fel(
                    AllwinnerFelRequest {
                        method: transition.method,
                        handle: transition.handle,
                        confirm_destructive: request.confirm_destructive,
                        preserve_serial_monitor,
                        allow_unverified_serial_shell: transition.allow_unverified_serial_shell,
                    },
                    None,
                    "Allwinner 进入 FEL 并烧录",
                )
                .await?;
            if !outcome.online {
                return Err(
                    "ALLWINNER_FEL_NOT_STABLE:fixed transition recipe did not produce two consecutive observations of the bound FEL/FES target"
                        .to_owned(),
                );
            }
            let (fel_usb_speed, _) = crate::workbench::flasher::usb_diagnostics(&scope.usb_binding);
            ensure_reliable_flash_link(
                &scope.chip_vendor,
                &scope.flasher_kind,
                &scope.usb_binding,
                &fel_usb_speed,
                firmware_bytes,
                request.allow_full_speed_large_image,
            )?;
            Some(outcome.hardware_guard)
        } else if let Some(transition) = request.rockchip_enter_loader {
            let outcome = self
                .execute_rockchip_enter_loader(
                    &scope,
                    RockchipLoaderRequest {
                        method: transition.method,
                        handle: transition.handle,
                        confirm_destructive: request.confirm_destructive,
                    },
                )
                .await?;
            self.ensure_rockchip_scope_unchanged(&scope)?;
            loader_transition = Some(outcome.response);
            Some(outcome.hardware_guard)
        } else {
            None
        };
        let task_id = crate::workbench::flasher::start_flash_with_args_and_guard(
            scope.device_id,
            request.firmware_path,
            override_args,
            app_handle,
            &workbench_state,
            crate::workbench::flasher::FlashAuditActor::Gateway,
            crate::workbench::flasher::StartFlashExecutionContext {
                hardware_guard,
                preserve_serial_monitor,
            },
        )?;

        Ok(json!({
            "taskId": task_id,
            "state": "running",
            "loaderTransition": loader_transition,
            "chipVendor": scope.chip_vendor,
            "flasherKind": scope.flasher_kind,
            "taskLifetime": "LYNX application process; independent of MCP sessions",
            "next": "Poll lynx_workbench_task_status with this taskId from this or any later MCP session until state is success, failed, or cancelled.",
            "warning": "Do not disconnect power or USB while the task is active."
        })
        .to_string())
    }

    #[tool(
        name = "lynx_allwinner_boot_mainline",
        description = "RAM BOOT STAGE: load an audited mainline U-Boot SPL, U-Boot proper, kernel, device tree and initramfs installer into this exact Allwinner board. This stage itself does not claim NAND success; the Linux installer must write through MTD/UBI and provide readback hashes before LYNX may report committed media bytes. Requires confirmDestructive=true and mandatory audit."
    )]
    async fn allwinner_boot_mainline(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            AllwinnerMainlineBootRequest,
        >,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_allwinner_boot_mainline requires a board channel".to_owned()
        })?;
        validate_allwinner_fel_scope(&scope.chip_vendor, &scope.flasher_kind)?;
        if scope.usb_binding.trim().is_empty() {
            return Err(
                "ALLWINNER_FEL_USB_UNBOUND:bind this board's FEL/FES USB interface first"
                    .to_owned(),
            );
        }
        crate::workbench::flasher::ensure_stable_allwinner_binding(
            &scope.flasher_kind,
            &scope.usb_binding,
        )?;
        // Hashing and loading a mainline plan can touch hundreds of MiB. Keep
        // it off the async MCP transport workers so initialize/status calls
        // remain responsive while a plan is prepared.
        let (artifacts, entry_address) =
            tokio::task::spawn_blocking(move || validate_mainline_plan(request))
                .await
                .map_err(|error| format!("MAINLINE_PLAN_WORKER_FAILED:{error}"))??;
        let artifacts = Arc::new(artifacts);
        let total_bytes = artifacts
            .iter()
            .map(|artifact| artifact.size_bytes)
            .sum::<u64>();
        let (usb_speed, _) = crate::workbench::flasher::usb_diagnostics(&scope.usb_binding);
        ensure_reliable_flash_link(
            &scope.chip_vendor,
            &scope.flasher_kind,
            &scope.usb_binding,
            &usb_speed,
            total_bytes,
            false,
        )?;
        let app_handle = self.app_handle.clone().ok_or_else(|| {
            "Mainline FEL boot requires the desktop-hosted LYNX Gateway.".to_owned()
        })?;
        let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
        let hardware_guard = safety.try_begin_scoped(
            "Linux MCP",
            "Allwinner 主线 FEL 启动",
            format!("device:{}", scope.device_id),
            crate::workbench::flasher::flasher_resource_key(
                &scope.flasher_kind,
                &scope.usb_binding,
            ),
        )?;
        let task_id = format!(
            "mainline-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        publish_mainline_audit(
            &app_handle,
            &scope,
            &task_id,
            artifacts.as_slice(),
            entry_address,
            MainlineAuditOutcome {
                action: "mainline_boot_intended",
                result: "authorized",
                error: None,
            },
        )?;
        self.record_device_interaction(
            "lynx_allwinner_boot_mainline",
            "烧录",
            format!(
                "AI 请求向固定 Allwinner FEL 单板写入 {} 个已校验主线 Artifact 并执行声明入口",
                artifacts.len()
            ),
            format!("usb:{}", scope.usb_binding),
        );

        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let workbench_state = workbench.inner().clone();
        let executable = crate::workbench::flasher::resolve_executable(
            &app_handle,
            &scope.flasher_kind,
            &scope.flasher_path,
        )?;
        let usb_location = scope
            .usb_binding
            .strip_prefix("location:")
            .ok_or_else(|| "FLASH_TARGET_LOCATION_REQUIRED:rescan and save this board".to_owned())?
            .to_owned();
        let device_lock = workbench.device_lock(scope.device_id)?;
        let flasher_lock =
            workbench.flasher_target_lock(&scope.flasher_kind, &scope.usb_binding)?;
        let cancellation = Arc::new(AtomicBool::new(false));
        insert_mainline_task(
            &workbench_state,
            &task_id,
            scope.device_id,
            artifacts.len(),
            total_bytes,
        )?;
        workbench
            .cancellations
            .lock()
            .map_err(|error| error.to_string())?
            .insert(task_id.clone(), cancellation.clone());

        let operation_app = app_handle.clone();
        let operation_scope = scope.clone();
        let operation_artifacts = artifacts.clone();
        let operation_task_id = task_id.clone();
        let operation_workbench = workbench_state.clone();
        let operation_cancellation = cancellation.clone();
        let spawn_result = std::thread::Builder::new()
            .name(format!("lynx-mainline-{}", scope.device_id))
            .spawn(move || {
                let _hardware_guard = hardware_guard;
                let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), String> {
                        ensure_mainline_not_cancelled(&operation_cancellation)?;
                        let _device_guard = device_lock.try_lock().map_err(|_| {
                            "DEVICE_BUSY:another operation is using this saved board".to_owned()
                        })?;
                        let _flasher_guard = flasher_lock.try_lock().map_err(|_| {
                            "FLASHER_BUSY:openixcli is performing another hardware operation"
                                .to_owned()
                        })?;
                        // The saved location is the board identity. Resolve it
                        // only after both locks are held so the worker receives
                        // the current ephemeral endpoint from this exact scan.
                        let endpoint = scan_current_allwinner_endpoint(&usb_location)?;
                        update_mainline_task_progress(
                            &operation_workbench,
                            &operation_task_id,
                            "opening_device",
                            0,
                            total_bytes,
                        );
                        run_mainline_worker(
                            &operation_app,
                            &executable,
                            &usb_location,
                            endpoint,
                            operation_artifacts.as_slice(),
                            entry_address,
                            total_bytes,
                            &operation_cancellation,
                            &operation_workbench,
                            &operation_task_id,
                        )?;
                        monitor_mainline_installer(
                            &operation_app,
                            &operation_scope.serial_port,
                            &operation_cancellation,
                            &operation_workbench,
                            &operation_task_id,
                        )
                    },
                ))
                .unwrap_or_else(|_| Err("MAINLINE_BOOT_WORKER_PANIC".to_owned()));

                let cancelled = operation_cancellation.load(Ordering::Relaxed);
                let (status, result, mut error) = match operation {
                    Ok(()) => ("success", "success", String::new()),
                    Err(error) if cancelled || error.starts_with("TASK_CANCELLED:") => {
                        ("cancelled", "cancelled", error)
                    }
                    Err(error) => ("failed", "failed", error),
                };
                if let Err(audit_error) = publish_mainline_audit(
                    &operation_app,
                    &operation_scope,
                    &operation_task_id,
                    operation_artifacts.as_slice(),
                    entry_address,
                    MainlineAuditOutcome {
                        action: "mainline_boot_completed",
                        result,
                        error: (!error.is_empty()).then_some(error.as_str()),
                    },
                ) {
                    error =
                        format!("AUDIT_RESULT_WRITE_FAILED:hardwareStatus={status}:{audit_error}");
                }
                let final_status = if error.starts_with("AUDIT_RESULT_WRITE_FAILED:") {
                    "failed"
                } else {
                    status
                };
                finish_mainline_task(
                    &operation_workbench,
                    &operation_task_id,
                    final_status,
                    &error,
                );
                if let Ok(mut tasks) = operation_workbench.cancellations.lock() {
                    tasks.remove(&operation_task_id);
                }
                crate::task_lifecycle::publish_workbench_completion(
                    &operation_app,
                    crate::task_lifecycle::WorkbenchTaskCompletion {
                        task_id: operation_task_id,
                        device_id: operation_scope.device_id,
                        kind: "mainline_boot".to_owned(),
                        status: final_status.to_owned(),
                        error,
                    },
                );
            });
        if let Err(spawn_error) = spawn_result {
            let mut error = format!("MAINLINE_WORKER_THREAD_START:{spawn_error}");
            if let Err(audit_error) = publish_mainline_audit(
                &app_handle,
                &scope,
                &task_id,
                artifacts.as_slice(),
                entry_address,
                MainlineAuditOutcome {
                    action: "mainline_boot_completed",
                    result: "failed",
                    error: Some(&error),
                },
            ) {
                error = format!("AUDIT_RESULT_WRITE_FAILED:hardwareStatus=failed:{audit_error}");
            }
            finish_mainline_task(&workbench_state, &task_id, "failed", &error);
            if let Ok(mut tasks) = workbench_state.cancellations.lock() {
                tasks.remove(&task_id);
            }
            crate::task_lifecycle::publish_workbench_completion(
                &app_handle,
                crate::task_lifecycle::WorkbenchTaskCompletion {
                    task_id: task_id.clone(),
                    device_id: scope.device_id,
                    kind: "mainline_boot".to_owned(),
                    status: "failed".to_owned(),
                    error: error.clone(),
                },
            );
            return Err(error);
        }

        Ok(json!({
            "taskId": task_id,
            "state": "running",
            "deviceId": scope.device_id,
            "usbBinding": scope.usb_binding,
            "artifactCount": artifacts.len(),
            "totalBytes": total_bytes,
            "entryAddress": format!("0x{entry_address:08x}"),
            "audit": "required_and_recorded",
            "next": "Poll lynx_workbench_task_status with this taskId until terminal. Use lynx_workbench_cancel_task to terminate the isolated helper; no automatic retry is performed.",
            "warning": "Do not disconnect power or USB while the task is active. Device opening is limited to 20 seconds; stalled transfers are limited to 30 seconds and release the hardware lock after helper termination."
        })
        .to_string())
    }

    #[tool(
        name = "lynx_serial_list",
        description = "Return the board-scoped serial binding, authoritative online state, and matching port details."
    )]
    async fn serial_list(&self) -> Result<String, String> {
        self.record_interaction("lynx_serial_list", "串口", "AI 查询串口列表");
        let _hardware_guard = if let Some(app_handle) = self.app_handle.as_ref() {
            crate::hardware_safety::ensure_idle(app_handle, "Linux MCP USB 串口扫描")?;
            let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
            Some(safety.try_begin("Linux MCP", "USB 串口扫描", "serial")?)
        } else {
            None
        };
        let scope = self.live_channel_scope();
        let ports = serialport::available_ports().map_err(|error| error.to_string())?;
        let result: Vec<_> = ports
            .into_iter()
            .filter(|port| {
                scope
                    .as_ref()
                    .is_none_or(|scope| port.port_name == scope.serial_port)
            })
            .map(|port| {
                json!({
                    "port": port.port_name,
                    "type": format!("{:?}", port.port_type)
                })
            })
            .collect();
        let configured_port = scope.as_ref().map(|scope| scope.serial_port.as_str());
        let online = !result.is_empty();
        serde_json::to_string_pretty(&json!({
            "configuredPort": configured_port,
            "online": online,
            "state": if configured_port.is_some_and(str::is_empty) { "unbound" } else if online { "online" } else { "not_found" },
            "ports": result,
        }))
        .map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_serial_open",
        description = "Open a serial debug port using 8 data bits, no parity, one stop bit, and no flow control."
    )]
    async fn serial_open(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            OpenSerialRequest,
        >,
    ) -> Result<String, String> {
        self.ensure_serial_scope(&request.port)?;
        self.record_device_interaction(
            "lynx_serial_open",
            "串口",
            format!("AI 打开串口 {} @ {}", request.port, request.baud_rate),
            format!("serial:{}", request.port),
        );
        if request.baud_rate == 0 {
            return Err("Baud rate must be greater than zero".to_string());
        }
        let fallback_port = if let Some(app_handle) = &self.app_handle {
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            if !crate::serial::commands::serial_is_open_core(
                serial_state.inner().as_ref(),
                &request.port,
            ) {
                crate::serial::commands::serial_open_core(
                    serial_state.inner().as_ref(),
                    app_handle.clone(),
                    request.port.clone(),
                    request.baud_rate,
                    8,
                    1,
                    "none".to_owned(),
                    "none".to_owned(),
                    None,
                )?;
            }
            None
        } else {
            Some(
                serialport::new(&request.port, request.baud_rate)
                    .data_bits(serialport::DataBits::Eight)
                    .parity(serialport::Parity::None)
                    .stop_bits(serialport::StopBits::One)
                    .flow_control(serialport::FlowControl::None)
                    .timeout(Duration::from_millis(250))
                    .open()
                    .map_err(|error| error.to_string())?,
            )
        };
        let id = self.serial.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = format!("serial-{id}");
        if let Some(port) = fallback_port {
            self.serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?
                .insert(handle.clone(), port);
        }
        self.serial
            .handle_ports
            .lock()
            .map_err(|error| error.to_string())?
            .insert(handle.clone(), request.port.clone());
        self.serial
            .handle_bauds
            .lock()
            .map_err(|error| error.to_string())?
            .insert(handle.clone(), request.baud_rate);
        Ok(json!({
            "handle": handle,
            "port": request.port,
            "baudRate": request.baud_rate,
            "settings": "8N1"
        })
        .to_string())
    }

    #[tool(
        name = "lynx_serial_read",
        description = "Read currently available data from an open serial debug session."
    )]
    async fn serial_read(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            ReadSerialRequest,
        >,
    ) -> Result<String, String> {
        let serial_device_key = self
            .serial
            .handle_ports
            .lock()
            .ok()
            .and_then(|items| items.get(&request.handle).cloned())
            .map(|port| format!("serial:{port}"))
            .unwrap_or_default();
        self.record_device_interaction(
            "lynx_serial_read",
            "串口",
            format!("AI 读取串口会话 {}", request.handle),
            serial_device_key,
        );
        let max_bytes = request.max_bytes.unwrap_or(4096).clamp(1, 65_536);
        let timeout = request.timeout_ms.unwrap_or(250).clamp(1, 5_000);
        let (serial_port, baud_rate) = self.serial_binding(&request.handle)?;
        let (buffer, connected) = if let Some(app_handle) = &self.app_handle {
            self.ensure_hosted_serial_open(app_handle, &serial_port, baud_rate)?;
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            let deadline = Instant::now() + Duration::from_millis(timeout);
            let mut quiet_deadline = None;
            let mut buffer = Vec::with_capacity(max_bytes.min(4096));
            loop {
                let data = crate::serial::commands::serial_read_core(
                    serial_state.inner().as_ref(),
                    &serial_port,
                    max_bytes.saturating_sub(buffer.len()),
                )?;
                if !data.is_empty() {
                    buffer.extend_from_slice(&data);
                    quiet_deadline = Some(Instant::now() + SERIAL_MCP_QUIET_WINDOW);
                }
                let now = Instant::now();
                if buffer.len() >= max_bytes
                    || now >= deadline
                    || quiet_deadline.is_some_and(|quiet| now >= quiet)
                {
                    break;
                }
                tokio::time::sleep(SERIAL_MCP_POLL_INTERVAL).await;
            }
            let connected = crate::serial::commands::serial_is_open_core(
                serial_state.inner().as_ref(),
                &serial_port,
            );
            (buffer, connected)
        } else {
            let mut ports = self
                .serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?;
            let port = ports.get_mut(&request.handle).ok_or_else(|| {
                format!(
                    "SERIAL_HANDLE_EXPIRED:{}:call lynx_serial_open",
                    request.handle
                )
            })?;
            port.set_timeout(Duration::from_millis(timeout))
                .map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; max_bytes];
            let count = match port.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => 0,
                Err(error) => return Err(error.to_string()),
            };
            buffer.truncate(count);
            (buffer, true)
        };
        let count = buffer.len();
        Ok(json!({
            "handle": request.handle,
            "bytesRead": count,
            "connected": connected,
            "port": serial_port,
            "text": String::from_utf8_lossy(&buffer),
            "hex": buffer.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        })
        .to_string())
    }

    #[tool(
        name = "lynx_serial_write",
        description = "Write UTF-8 text to an open serial debug session."
    )]
    async fn serial_write(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            WriteSerialRequest,
        >,
    ) -> Result<String, String> {
        let serial_device_key = self
            .serial
            .handle_ports
            .lock()
            .ok()
            .and_then(|items| items.get(&request.handle).cloned())
            .map(|port| format!("serial:{port}"))
            .unwrap_or_default();
        self.record_device_interaction(
            "lynx_serial_write",
            "串口",
            format!(
                "AI 写入串口会话 {}（{} 字节）",
                request.handle,
                request.data.len()
            ),
            serial_device_key,
        );
        if request.data.len() > 65_536 {
            return Err("Serial write is limited to 65536 bytes".to_string());
        }
        validate_ai_serial_command(
            &request.data,
            self.live_channel_scope()
                .as_ref()
                .map(|scope| scope.chip_vendor.as_str()),
        )?;
        let mut data = request.data.into_bytes();
        if request.append_newline.unwrap_or(false) {
            data.push(b'\n');
        }
        self.write_serial_bytes(&request.handle, &data)?;
        Ok(json!({"handle": request.handle, "bytesWritten": data.len()}).to_string())
    }

    fn write_serial_bytes(&self, handle: &str, data: &[u8]) -> Result<(), String> {
        let (serial_port, baud_rate) = self.serial_binding(handle)?;
        if let Some(app_handle) = &self.app_handle {
            self.ensure_hosted_serial_open(app_handle, &serial_port, baud_rate)?;
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            crate::serial::commands::serial_write_core(
                serial_state.inner().as_ref(),
                &serial_port,
                data,
            )?;
        } else {
            let mut ports = self
                .serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?;
            let port = ports
                .get_mut(handle)
                .ok_or_else(|| format!("SERIAL_HANDLE_EXPIRED:{handle}:call lynx_serial_open"))?;
            port.write_all(data).map_err(|error| error.to_string())?;
            port.flush().map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn read_serial_bytes(
        &self,
        handle: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let (serial_port, baud_rate) = self.serial_binding(handle)?;
        if let Some(app_handle) = &self.app_handle {
            self.ensure_hosted_serial_open(app_handle, &serial_port, baud_rate)?;
            tokio::time::sleep(timeout.min(Duration::from_millis(150))).await;
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            crate::serial::commands::serial_read_core(
                serial_state.inner().as_ref(),
                &serial_port,
                max_bytes,
            )
        } else {
            let mut ports = self
                .serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?;
            let port = ports
                .get_mut(handle)
                .ok_or_else(|| format!("SERIAL_HANDLE_EXPIRED:{handle}:call lynx_serial_open"))?;
            port.set_timeout(timeout)
                .map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; max_bytes];
            let count = match port.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => 0,
                Err(error) => return Err(error.to_string()),
            };
            buffer.truncate(count);
            Ok(buffer)
        }
    }

    fn release_serial_handles_for_port(
        &self,
        handle: &str,
        serial_port: &str,
    ) -> Result<usize, String> {
        let mut released_handles = self
            .serial
            .handle_ports
            .lock()
            .map_err(|error| error.to_string())?
            .iter()
            .filter(|(_, port)| port.as_str() == serial_port)
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        if released_handles.is_empty() {
            released_handles.push(handle.to_owned());
        }
        if let Some(app_handle) = &self.app_handle {
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            // The K230 is expected to disappear while switching USB modes, so
            // an already-gone serial device is a successful transition only
            // when no shared handle remains. Every other close failure must be
            // visible instead of claiming `serialClosed: true`.
            if crate::serial::commands::serial_is_registered_core(
                serial_state.inner().as_ref(),
                serial_port,
            ) {
                crate::serial::commands::serial_close_core(
                    serial_state.inner().as_ref(),
                    serial_port,
                )?;
            }
        } else {
            let mut ports = self
                .serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?;
            for released_handle in &released_handles {
                ports.remove(released_handle);
            }
        }
        let mut items = self
            .serial
            .handle_ports
            .lock()
            .map_err(|error| error.to_string())?;
        for released_handle in &released_handles {
            items.remove(released_handle);
        }
        drop(items);
        let mut items = self
            .serial
            .handle_bauds
            .lock()
            .map_err(|error| error.to_string())?;
        for released_handle in &released_handles {
            items.remove(released_handle);
        }
        Ok(released_handles.len())
    }

    async fn wait_for_bound_flash_device(
        &self,
        scope: &ChannelScope,
        timeout: Duration,
        scan_error_code: &str,
    ) -> Result<bool, String> {
        let Some(app_handle) = self.app_handle.clone() else {
            return Ok(false);
        };
        let started = Instant::now();
        let mut last_scan_error = None;
        let mut consecutive_matches = 0_u8;
        loop {
            let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
            let device_lock = workbench.device_lock(scope.device_id)?;
            let flasher_lock =
                workbench.flasher_target_lock(&scope.flasher_kind, &scope.usb_binding)?;
            let scan_app = app_handle.clone();
            let kind = scope.flasher_kind.clone();
            let path = scope.flasher_path.clone();
            let binding = scope.usb_binding.clone();
            let scan_result = tauri::async_runtime::spawn_blocking(move || {
                let Ok(_device_guard) = device_lock.try_lock() else {
                    return Ok(false);
                };
                let Ok(_flasher_guard) = flasher_lock.try_lock() else {
                    return Ok(false);
                };
                crate::workbench::flasher::scan_flasher_devices(&scan_app, &kind, &path).map(
                    |devices| {
                        let matches = devices
                            .iter()
                            .filter(|device| {
                                if kind == "openixcli" {
                                    crate::workbench::flasher::allwinner_binding_matches(
                                        &binding,
                                        &device.key,
                                    )
                                } else {
                                    device.key == binding
                                }
                            })
                            .count();
                        matches == 1
                    },
                )
            })
            .await
            .map_err(|error| error.to_string())?;
            match scan_result {
                Ok(true) => {
                    consecutive_matches += 1;
                    if consecutive_matches >= 2 {
                        return Ok(true);
                    }
                }
                Ok(false) => consecutive_matches = 0,
                Err(error) => {
                    consecutive_matches = 0;
                    last_scan_error = Some(error);
                }
            }
            if started.elapsed() >= timeout {
                return last_scan_error
                    .map_or(Ok(false), |error| Err(format!("{scan_error_code}:{error}")));
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    }

    #[tool(
        name = "lynx_rockchip_enter_loader",
        description = "REBOOT: enter Loader (not Maskrom) on this saved Rockchip/openrockcli board. Requires confirmDestructive=true. Use adb_reboot_loader with the saved ADB binding, or serial_shell_reboot_loader with a lynx_serial_open handle at the saved Linux shell prompt. Sends only fixed reboot loader, closes the serial handle after transmission, and requires two OpenRockCLI Loader observations at the saved USB location. Timeout stops and requests physical Maskrom entry; no automatic retry. For uninterrupted transition and flashing use lynx_start_flash.rockchipEnterLoader instead."
    )]
    async fn rockchip_enter_loader(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<RockchipLoaderRequest>,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_rockchip_enter_loader requires a board channel".to_owned()
        })?;
        Ok(self
            .execute_rockchip_enter_loader(&scope, request)
            .await?
            .response
            .to_string())
    }

    #[tool(
        name = "lynx_allwinner_enter_fel",
        description = "For a board-scoped Allwinner/openixcli channel only, enter FEL through one of three fixed recipes: adb_reboot_efex sends only the compiled-in `reboot efex`; uboot_serial_efex uses the configured power relay only to cycle board power, interrupts the bound U-Boot console, waits for `=>`, then sends only `efex`; serial_shell_reboot_efex first proves the bound console is at the saved Linux shell prompt, then sends only the compiled-in `reboot efex`. Set preserveSerialMonitor=true only when the existing serial handle must remain readable during vendor U-Boot/FES diagnostics. The physical FEL button is never mapped to a relay and three GPIOs are not required. Requires confirmDestructive=true and verifies the bound FEL/FES USB device before reporting it online."
    )]
    async fn allwinner_enter_fel(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            AllwinnerFelRequest,
        >,
    ) -> Result<String, String> {
        Ok(self
            .execute_allwinner_enter_fel(request, None, "Allwinner 进入 FEL")
            .await?
            .response)
    }

    async fn execute_allwinner_enter_fel(
        &self,
        request: AllwinnerFelRequest,
        existing_guard: Option<crate::hardware_safety::HardwareActivityGuard>,
        operation: &str,
    ) -> Result<AllwinnerFelOutcome, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_allwinner_enter_fel requires a board channel".to_owned()
        })?;
        validate_allwinner_fel_scope(&scope.chip_vendor, &scope.flasher_kind)?;
        if !request.confirm_destructive {
            return Err(
                "CONFIRMATION_REQUIRED:confirmDestructive=true is required to reboot or power-cycle this Allwinner board"
                    .to_owned(),
            );
        }
        if scope.usb_binding.trim().is_empty() {
            return Err(
                "ALLWINNER_FEL_USB_UNBOUND:bind this board's FEL/FES USB interface before entering FEL"
                    .to_owned(),
            );
        }
        crate::workbench::flasher::ensure_stable_allwinner_binding(
            &scope.flasher_kind,
            &scope.usb_binding,
        )?;
        let app_handle = self.app_handle.as_ref().ok_or_else(|| {
            "Allwinner FEL entry requires the desktop-hosted LYNX Gateway.".to_owned()
        })?;
        let hardware_guard = match existing_guard {
            Some(guard) => guard,
            None => {
                let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
                safety.try_begin_scoped(
                    "Linux MCP",
                    operation,
                    format!("device:{}", scope.device_id),
                    crate::workbench::flasher::flasher_resource_key(
                        &scope.flasher_kind,
                        &scope.usb_binding,
                    ),
                )?
            }
        };

        let (method, serial_port, serial_handles_closed, prompt_observed) = match request.method {
            AllwinnerFelMethod::AdbRebootEfex => {
                if scope.adb_serial.trim().is_empty() {
                    return Err(format!(
                        "DEVICE_SCOPE:单板 {} 未绑定 ADB，不能使用 adb_reboot_efex",
                        scope.device_id
                    ));
                }
                self.record_device_interaction(
                    "lynx_allwinner_enter_fel",
                    "ADB",
                    "AI 通过受控固定 Recipe 请求 Allwinner 进入 FEL".to_owned(),
                    format!("adb:{}", scope.adb_serial),
                );
                crate::adb::commands::adb_shell_command_core(
                    app_handle,
                    &app_handle.state::<crate::adb::commands::AdbState>(),
                    Some(scope.adb_serial.clone()),
                    ALLWINNER_ADB_EFEX_COMMAND.to_owned(),
                    crate::adb::commands::AdbOperationActor::Gateway,
                )
                .map_err(|error| format!("ALLWINNER_ADB_EFEX_FAILED:{}", error.name))?;
                ("adb_reboot_efex", None, 0, true)
            }
            AllwinnerFelMethod::UbootSerialEfex => {
                let handle = request.handle.as_deref().ok_or_else(|| {
                    "SERIAL_HANDLE_REQUIRED:uboot_serial_efex requires a handle returned by lynx_serial_open"
                        .to_owned()
                })?;
                let (serial_port, _) = self.serial_binding(handle)?;
                self.ensure_serial_scope(&serial_port)?;
                let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
                let device = {
                    let db = workbench.db.lock().map_err(|error| error.to_string())?;
                    crate::workbench::storage::device_by_id(&db, scope.device_id)
                        .map_err(|error| error.to_string())?
                };
                self.record_device_interaction(
                    "lynx_allwinner_enter_fel",
                    "串口",
                    format!(
                        "AI 通过电源重启与绑定串口 {} 请求 Allwinner 进入 FEL",
                        serial_port
                    ),
                    format!("serial:{serial_port}"),
                );

                // The application-owned serial reader survives page/tool
                // sessions. Discard an old U-Boot prompt before cycling power;
                // otherwise a stale `=>` can make the recipe send efex before
                // the new U-Boot instance is ready, while falsely reporting
                // that prompt detection succeeded.
                for _ in 0..ALLWINNER_STALE_DRAIN_PASSES {
                    let stale = self
                        .read_serial_bytes(handle, 65_536, Duration::from_millis(20))
                        .await?;
                    if stale.len() < 65_536 {
                        break;
                    }
                }

                // The relay controls board power only. It never represents or
                // attempts to press the board's physical FEL button.
                crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOff,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?;
                tokio::time::sleep(Duration::from_millis(
                    device.power_off_wait_ms.clamp(100, 30_000),
                ))
                .await;
                crate::workbench::power::execute_and_audit(
                    app_handle,
                    &workbench,
                    None,
                    &device,
                    crate::workbench::types::PowerAction::PowerOn,
                    0,
                    crate::workbench::power::PowerAuditActor::Gateway,
                )?;

                let started = Instant::now();
                let mut console = Vec::new();
                let mut last_serial_error: Option<String>;
                loop {
                    match self.write_serial_bytes(handle, ALLWINNER_FEL_INTERRUPT) {
                        Ok(()) => {
                            match self
                                .read_serial_bytes(handle, 4096, Duration::from_millis(120))
                                .await
                            {
                                Ok(chunk) => {
                                    console.extend_from_slice(&chunk);
                                    if contains_allwinner_uboot_prompt(&console) {
                                        break;
                                    }
                                    last_serial_error = None;
                                }
                                Err(error) => last_serial_error = Some(error),
                            }
                        }
                        Err(error) => last_serial_error = Some(error),
                    }
                    if console.len() > 65_536 {
                        console.drain(..console.len() - 65_536);
                    }
                    if started.elapsed() >= ALLWINNER_UBOOT_PROMPT_TIMEOUT {
                        let serial_error = last_serial_error.as_deref().map(|error| {
                            crate::audit::machine_error_code(error, "SERIAL_READ_FAILED")
                        });
                        return Err(format!(
                            "ALLWINNER_UBOOT_PROMPT_TIMEOUT:bytesObserved={}:serialError={}:did not send efex",
                            console.len(),
                            serial_error.as_deref().unwrap_or("none")
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                self.write_serial_bytes(handle, ALLWINNER_UBOOT_EFEX_COMMAND)?;
                self.record_device_interaction(
                    "lynx_allwinner_enter_fel",
                    "串口",
                    format!(
                        "已识别当前 U-Boot 提示符并写入固定 efex Recipe（{} 字节）",
                        ALLWINNER_UBOOT_EFEX_COMMAND.len()
                    ),
                    format!("serial:{serial_port}"),
                );
                // write_all + flush transfers into the OS/USB serial stack,
                // not necessarily onto the wire. Keep the handle alive for a
                // bounded transmit window. The caller may explicitly retain
                // it as a read monitor for vendor U-Boot/FES diagnostics.
                tokio::time::sleep(ALLWINNER_EFEX_TX_SETTLE).await;
                let closed = if request.preserve_serial_monitor {
                    0
                } else {
                    self.release_serial_handles_for_port(handle, &serial_port)
                        .map_err(|error| {
                            format!("ALLWINNER_SERIAL_RELEASE_FAILED:{serial_port}:{error}")
                        })?
                };
                ("uboot_serial_efex", Some(serial_port), closed, true)
            }
            AllwinnerFelMethod::SerialShellRebootEfex => {
                let handle = request.handle.as_deref().ok_or_else(|| {
                    "SERIAL_HANDLE_REQUIRED:serial_shell_reboot_efex requires a handle returned by lynx_serial_open"
                        .to_owned()
                })?;
                let (serial_port, _) = self.serial_binding(handle)?;
                self.ensure_serial_scope(&serial_port)?;
                let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
                let device = {
                    let db = workbench.db.lock().map_err(|error| error.to_string())?;
                    crate::workbench::storage::device_by_id(&db, scope.device_id)
                        .map_err(|error| error.to_string())?
                };
                let shell_prompt = device.serial_shell_prompt.trim();
                if shell_prompt.is_empty() {
                    return Err(
                        "ALLWINNER_SHELL_PROMPT_UNCONFIGURED:configure the board serial shell prompt before using serial_shell_reboot_efex"
                            .to_owned(),
                    );
                }
                self.record_device_interaction(
                    "lynx_allwinner_enter_fel",
                    "串口",
                    format!(
                        "AI 通过绑定串口 {} 的已验证 Linux Shell 请求 Allwinner 进入 FEL",
                        serial_port
                    ),
                    format!("serial:{serial_port}"),
                );

                // Prove that this exact board console is already at its saved
                // Linux shell prompt before sending the compiled-in reboot
                // recipe. Login automation and caller-provided commands are
                // intentionally outside this transition.
                self.write_serial_bytes(handle, b"\r\n")?;
                let started = Instant::now();
                let mut console = Vec::new();
                let prompt_observed;
                loop {
                    let chunk = self
                        .read_serial_bytes(handle, 4096, Duration::from_millis(150))
                        .await?;
                    console.extend_from_slice(&chunk);
                    if contains_serial_shell_prompt(&console, shell_prompt) {
                        prompt_observed = true;
                        break;
                    }
                    if console.len() > 65_536 {
                        console.drain(..console.len() - 65_536);
                    }
                    if started.elapsed() >= ALLWINNER_SHELL_PROMPT_TIMEOUT {
                        if request.allow_unverified_serial_shell {
                            prompt_observed = false;
                            break;
                        }
                        return Err(format!(
                            "ALLWINNER_SHELL_PROMPT_TIMEOUT:bytesObserved={}:did not send reboot efex",
                            console.len()
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                self.write_serial_bytes(handle, ALLWINNER_SERIAL_SHELL_EFEX_COMMAND)?;
                self.record_device_interaction(
                    "lynx_allwinner_enter_fel",
                    "串口",
                    format!(
                        "写入固定 reboot efex Recipe（{} 字节，promptObserved={}）",
                        ALLWINNER_SERIAL_SHELL_EFEX_COMMAND.len(),
                        prompt_observed
                    ),
                    format!("serial:{serial_port}"),
                );
                tokio::time::sleep(ALLWINNER_EFEX_TX_SETTLE).await;
                let closed = if request.preserve_serial_monitor {
                    0
                } else {
                    self.release_serial_handles_for_port(handle, &serial_port)
                        .map_err(|error| {
                            format!("ALLWINNER_SERIAL_RELEASE_FAILED:{serial_port}:{error}")
                        })?
                };
                (
                    "serial_shell_reboot_efex",
                    Some(serial_port),
                    closed,
                    prompt_observed,
                )
            }
        };

        let fel_online = self
            .wait_for_bound_flash_device(
                &scope,
                ALLWINNER_FEL_USB_TIMEOUT,
                "ALLWINNER_FEL_SCAN_TIMEOUT",
            )
            .await?;
        let response = json!({
            "deviceId": scope.device_id,
            "method": method,
            "port": serial_port,
            "serialHandlesClosed": serial_handles_closed,
            "serialMonitorRetained": request.preserve_serial_monitor && serial_port.is_some(),
            "promptObserved": prompt_observed,
            "efexBytesWritten": match method {
                "uboot_serial_efex" => ALLWINNER_UBOOT_EFEX_COMMAND.len(),
                "serial_shell_reboot_efex" => ALLWINNER_SERIAL_SHELL_EFEX_COMMAND.len(),
                _ => 0,
            },
            "felOnline": fel_online,
            "state": if fel_online { "fel-device-online" } else { "fel-device-not-found" },
            "next": if fel_online {
                "The scoped Allwinner FEL/FES USB device is online; flashing may start."
            } else {
                "The fixed transition recipe completed, but the bound FEL/FES USB device did not appear. Do not start flashing and do not retry automatically."
            }
        })
        .to_string();
        Ok(AllwinnerFelOutcome {
            response,
            online: fel_online,
            hardware_guard,
        })
    }

    #[tool(
        name = "lynx_k230_enter_upgrade_mode",
        description = "For a board-scoped Kendryte K230/K230D channel only, interrupt the running console with Ctrl+C, wait for the `msh />` prompt, send the exact `reboot_to_upgrade` command, close the stale serial handle, and wait for the bound K230 USB Boot device. Use the handle returned by lynx_serial_open."
    )]
    async fn k230_enter_upgrade_mode(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            SerialHandleRequest,
        >,
    ) -> Result<String, String> {
        let scope = self.live_channel_scope().ok_or_else(|| {
            "CHANNEL_SCOPE:lynx_k230_enter_upgrade_mode requires a board channel".to_owned()
        })?;
        validate_k230_upgrade_scope(&scope.chip_vendor, &scope.flasher_kind)?;
        let app_handle = self.app_handle.as_ref().ok_or_else(|| {
            "K230 upgrade mode requires the desktop-hosted LYNX Gateway.".to_owned()
        })?;
        crate::hardware_safety::ensure_idle(app_handle, "K230 进入烧录模式")?;
        let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard = safety.try_begin(
            "Linux MCP",
            "K230 进入烧录模式",
            format!("device:{}", scope.device_id),
        )?;
        if scope.usb_binding.trim().is_empty() {
            return Err(
                "K230_UPGRADE_USB_UNBOUND:bind this board's K230 USB Boot interface before entering upgrade mode"
                    .to_owned(),
            );
        }

        let (serial_port, _) = self.serial_binding(&request.handle)?;
        self.ensure_serial_scope(&serial_port)?;
        self.record_device_interaction(
            "lynx_k230_enter_upgrade_mode",
            "串口",
            format!("AI 通过串口 {} 请求 K230 进入 USB 烧录模式", serial_port),
            format!("serial:{serial_port}"),
        );

        // Discard earlier boot logs before starting the transition. Otherwise
        // an old `msh />` left in the shared read buffer could be mistaken for
        // the prompt produced by this Ctrl+C sequence.
        for _ in 0..K230_STALE_DRAIN_PASSES {
            let stale = self
                .read_serial_bytes(&request.handle, 65_536, Duration::from_millis(20))
                .await?;
            if stale.len() < 65_536 {
                break;
            }
        }
        self.write_serial_bytes(&request.handle, K230_UPGRADE_INTERRUPT)?;
        let started = Instant::now();
        let mut console = Vec::new();
        let mut prompt_nudged = false;
        loop {
            let chunk = self
                .read_serial_bytes(&request.handle, 4096, Duration::from_millis(150))
                .await?;
            console.extend_from_slice(&chunk);
            if console
                .windows(K230_UPGRADE_PROMPT.len())
                .any(|window| window == K230_UPGRADE_PROMPT)
            {
                break;
            }
            // A blank line makes MSH redraw its prompt after the application
            // has exited. It is intentionally CRLF and contains no 0xff byte.
            if !prompt_nudged
                && (console
                    .windows(b"desktop worker".len())
                    .any(|window| window == b"desktop worker")
                    || started.elapsed() >= Duration::from_millis(800))
            {
                self.write_serial_bytes(&request.handle, b"\r\n")?;
                prompt_nudged = true;
            }
            if console.len() > 65_536 {
                console.drain(..console.len() - 65_536);
            }
            if started.elapsed() >= K230_UPGRADE_PROMPT_TIMEOUT {
                return Err(format!(
                    "K230_MSH_PROMPT_TIMEOUT:bytesObserved={}:did not send reboot_to_upgrade",
                    console.len()
                ));
            }
        }

        self.write_serial_bytes(&request.handle, K230_UPGRADE_COMMAND)?;
        // Release the Windows COM handle immediately after write_all + flush.
        // The target starts re-enumerating as soon as it receives CRLF; keeping
        // the stale handle alive even briefly can leave usbser reporting
        // ERROR_INVALID_FUNCTION until the adapter is physically unplugged.
        let serial_handles_closed = self
            .release_serial_handles_for_port(&request.handle, &serial_port)
            .map_err(|error| format!("K230_SERIAL_RELEASE_FAILED:{serial_port}:{error}"))?;
        let usb_online = self
            .wait_for_bound_flash_device(&scope, K230_UPGRADE_USB_TIMEOUT, "K230_USB_SCAN_TIMEOUT")
            .await?;

        Ok(json!({
            "handle": request.handle,
            "port": serial_port,
            "command": "Ctrl+C, wait for msh />, then reboot_to_upgrade\\r\\n",
            "serialClosed": true,
            "serialHandlesClosed": serial_handles_closed,
            "usbOnline": usb_online,
            "state": if usb_online { "upgrade-device-online" } else { "upgrade-device-not-found" },
            "next": if usb_online {
                "The scoped K230 USB Boot device is online; flashing may start."
            } else {
                "The command was sent exactly, but the bound K230 USB Boot device did not appear before the scan timeout. Do not start flashing; inspect USB binding and driver state."
            }
        })
        .to_string())
    }

    #[tool(
        name = "lynx_serial_close",
        description = "Close a LYNX serial debug session."
    )]
    async fn serial_close(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<
            SerialHandleRequest,
        >,
    ) -> Result<String, String> {
        let serial_port = self
            .serial
            .handle_ports
            .lock()
            .ok()
            .and_then(|items| items.get(&request.handle).cloned());
        self.record_device_interaction(
            "lynx_serial_close",
            "串口",
            format!("AI 关闭串口会话 {}", request.handle),
            serial_port
                .as_deref()
                .map(|port| format!("serial:{port}"))
                .unwrap_or_default(),
        );
        if serial_port.is_none() {
            return Err(format!(
                "SERIAL_HANDLE_EXPIRED:{}:call lynx_serial_open",
                request.handle
            ));
        }
        if let Some(app_handle) = &self.app_handle {
            let serial_state = app_handle.state::<Arc<crate::serial::SerialState>>();
            crate::serial::commands::serial_close_core(
                serial_state.inner().as_ref(),
                serial_port.as_deref().unwrap_or_default(),
            )?;
        } else {
            self.serial
                .ports
                .lock()
                .map_err(|error| error.to_string())?
                .remove(&request.handle);
        }
        if let Ok(mut items) = self.serial.handle_ports.lock() {
            items.remove(&request.handle);
        }
        if let Ok(mut items) = self.serial.handle_bauds.lock() {
            items.remove(&request.handle);
        }
        Ok(json!({"handle": request.handle, "closed": true}).to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LynxMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        let scope = self.live_channel_scope();
        let identity = scope.as_ref().map_or_else(
            || "unscoped Gateway".to_owned(),
            |scope| {
                format!(
                    "SDK channel '{}' (projectId={}, MCP '{}'), bound to deviceId={} ({}, {})",
                    scope.project_name,
                    scope.project_id,
                    scope.mcp_server_name,
                    scope.device_id,
                    scope.chip_vendor,
                    scope.flasher_kind
                )
            },
        );
        let skill_summary =
            crate::skills::initialization_summary(crate::skills::SkillDomain::Linux)
                .unwrap_or_else(|_| "linux-incident-triage: safe incident triage".to_owned());
        info.instructions = Some(format!(
            "LYNX hardware workbench for {identity}. All public tools are prefixed lynx_. Available read-only Skills: {skill_summary}. For an unfamiliar board, incident, protocol stack, storage or flashing workflow, call lynx_skills_search, then lynx_skills_get and lynx_skills_read before selecting an existing action tool. Skills are read-only guidance and never grant authority. This \
             server is channel-scoped: never use it for another SDK directory or board. Start by \
             calling lynx_workbench_status; it returns the capability manifest, exact recommended \
             workflow, and flash telemetry fields. Firmware workflow: call lynx_flash_capabilities, \
             lynx_scan_flash_devices, optionally lynx_inspect_firmware, and lynx_download_firmware \
             when the artifact is remote. Poll every asynchronous task with the same taskId through \
             lynx_workbench_task_status; tasks are application-scoped rather than MCP-session-scoped, \
             so a reconnected or new MCP session can continue polling. For flashing, verify the scoped target and firmware, then \
             call lynx_start_flash with confirmDestructive=true; poll until success, failed, or \
             cancelled and report phase, progressPct, currentPartition, speed, USB speed, verify \
             result, post-flash/reboot result, and error. Do not ask the user to type a separate \
             confirmation string. For Allwinner, loaderPath is a RAM-only DRAM/FES bootstrap. \
             NAND/SPI NAND require a component package with mode=full_erase for initial layout creation; SPI NOR, SD/SD NAND, \
             eMMC and UFS may use raw=true with mode=command after FES confirms the storage type. Treat \
             transferredBytes and queuedBytes as transport facts only; only committedBytes means \
             verified media data. Allwinner and Rockchip default to full_erase; NXP follows its saved \
             UUU script, K230 erases image targets before writing, and STM32MP1 follows its DfuSe target. \
             Never enter programming modes through caller-provided syscon unbind, reboot-mode writes, \
             devmem, ADB/shell reboot commands, or another RAM/register hack. For an Allwinner/openixcli \
             channel, call lynx_allwinner_enter_fel with confirmDestructive=true and choose the fixed \
             adb_reboot_efex recipe, uboot_serial_efex using a bound serial handle, or \
             serial_shell_reboot_efex after proving the bound console is at its saved Linux shell prompt. The U-Boot recipe uses \
             the relay only to cycle board power, then sends Ctrl+C, waits for the U-Boot `=>` prompt, and \
             sends exact `efex`; it never treats the physical FEL button as a relay and never requires three GPIOs. \
             For Rockchip/openrockcli, use lynx_rockchip_enter_loader or the rockchipEnterLoader option \
             of lynx_start_flash with a fixed adb_reboot_loader or serial_shell_reboot_loader recipe. \
             The latter requires a bound serial handle and a fresh saved Linux shell prompt. \
             Only OpenRockCLI confirmation of the bound Loader permits flashing. Loader is not Maskrom; \
             timeout requires physical Maskrom intervention and must not trigger an automatic retry. \
             For a channel scoped to Kendryte K230/K230D, open its bound serial port and call \
             lynx_k230_enter_upgrade_mode. That tool sends \
             Ctrl+C, wait for `msh />`, send the vendor-supported `reboot_to_upgrade` command, then scan for the K230 USB device. \
             Power workflow: call lynx_power_capabilities before \
             lynx_power_control; lynx_power_status may be last-known for write-only Lynx Power controllers. \
             Serial handles come from lynx_serial_open and must be reused with read/write/close; on \
             SERIAL_HANDLE_EXPIRED reopen them. ADB access is limited to this channel's bound serial."
        ));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

fn validate_ai_shell_command(command: &str, chip_vendor: Option<&str>) -> Result<(), String> {
    let normalized = command.to_ascii_lowercase();
    let software_boot_transition = normalized.contains("devmem")
        || normalized.contains("reboot-mode")
        || (normalized.contains("syscon") && normalized.contains("unbind"))
        || normalized.contains("reboot bootloader")
        || normalized.contains("reboot loader")
        || normalized.contains("reboot maskrom")
        || normalized.contains("reboot fel")
        || normalized.contains("reboot efex")
        || normalized.contains("reboot fastboot")
        || normalized.contains("reboot brom")
        || normalized.contains("reboot u-boot")
        || normalized.contains("reboot uboot")
        || normalized.contains("reboot dfu");
    if software_boot_transition {
        let vendor = chip_vendor.unwrap_or("this board");
        return Err(
            format!("UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:{vendor} programming mode must use the dedicated board-scoped transition tool (Rockchip: lynx_rockchip_enter_loader), physical boot controls or a configured reliable hardware reset/power controller; syscon unbind, reboot-mode writes, devmem, and software reboot-to-programming-mode commands are prohibited"),
        );
    }
    Ok(())
}

fn validate_ai_serial_command(command: &str, chip_vendor: Option<&str>) -> Result<(), String> {
    validate_ai_shell_command(command, chip_vendor)?;
    let contains_efex = command
        .to_ascii_lowercase()
        .split(|character: char| character.is_ascii_whitespace() || ";|&".contains(character))
        .any(|token| token == "efex");
    if contains_efex {
        let vendor = chip_vendor.unwrap_or("unscoped serial");
        return Err(format!(
            "UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:{vendor} U-Boot efex must use the board-scoped lynx_allwinner_enter_fel recipe"
        ));
    }
    Ok(())
}

/// Run the loopback-only gateway as a foreground process.
///
/// This entry point is used by the companion `openix-gateway` binary so a
/// hardware host can serve Codex or Claude without opening the desktop UI.
#[allow(dead_code)]
pub async fn run_foreground(port: u16) -> anyhow::Result<()> {
    if port == 0 {
        anyhow::bail!("Gateway port must be between 1 and 65535");
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let cancellation_token = CancellationToken::new();
    let router = gateway_router(
        &cancellation_token,
        None,
        Arc::new(Mutex::new(VecDeque::with_capacity(200))),
        Arc::new(AtomicU64::new(0)),
        None,
    );
    let shutdown_token = cancellation_token.clone();

    println!("LYNX MCP Gateway listening at http://127.0.0.1:{port}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown_token.cancel();
        })
        .await?;
    Ok(())
}

/// Start the loopback-only MCP gateway.
#[tauri::command]
pub async fn gateway_start(
    app_handle: AppHandle,
    state: State<'_, GatewayState>,
    port: Option<u16>,
    project_id: Option<i64>,
    device_id: Option<i64>,
) -> Result<GatewayStatus, AppError> {
    start_gateway(app_handle, state.inner(), port, project_id, device_id).await
}

/// Start a device-scoped Gateway without requiring a frontend command.
///
/// The desktop setup path uses this to restore every saved SDK channel before
/// any page is opened.
pub(crate) async fn start_gateway(
    app_handle: AppHandle,
    state: &GatewayState,
    port: Option<u16>,
    project_id: Option<i64>,
    device_id: Option<i64>,
) -> Result<GatewayStatus, AppError> {
    let port = port.unwrap_or(DEFAULT_PORT);
    if port == 0 {
        return Err(AppError::internal(
            "Gateway port must be between 1 and 65535",
        ));
    }

    let channel_scope = if let Some(project_id) = project_id {
        let workbench = app_handle.state::<Arc<crate::workbench::WorkbenchState>>();
        let (project, device) = {
            let db = workbench
                .db
                .lock()
                .map_err(|error| AppError::internal(error.to_string()))?;
            let project = crate::workbench::storage::project_by_id(&db, project_id)
                .map_err(|error| AppError::internal(error.to_string()))?;
            let scoped_device_id = project.device_id.ok_or_else(|| {
                AppError::internal(format!("SDK channel {project_id} has no bound device"))
            })?;
            if device_id.is_some_and(|id| id != scoped_device_id) {
                return Err(AppError::internal(format!(
                    "SDK channel {project_id} is not bound to device {}",
                    device_id.unwrap_or_default()
                )));
            }
            let device = crate::workbench::storage::device_by_id(&db, scoped_device_id)
                .map_err(|error| AppError::internal(error.to_string()))?;
            (project, device)
        };
        let serial_port = live_serial_port(&device);
        Some(ChannelScope {
            project_id,
            project_name: project.name,
            mcp_server_name: project.mcp_server_name,
            device_id: device.id.unwrap_or_default(),
            chip_vendor: device.chip_vendor,
            flasher_kind: device.flasher_kind,
            flasher_path: device.flasher_path,
            serial_port,
            usb_binding: device.usb_binding,
            adb_serial: device.adb_serial,
            camera_binding: device.camera_binding,
        })
    } else if let Some(device_id) = device_id {
        return Err(AppError::internal(format!(
            "Device-scoped Gateway for device {device_id} requires projectId"
        )));
    } else {
        None
    };

    let previous_runtime = {
        let mut guard = state
            .runtimes
            .lock()
            .map_err(|error| AppError::internal(error.to_string()))?;
        if guard
            .get(&port)
            .is_some_and(|runtime| same_gateway_route(&runtime.channel_scope, &channel_scope))
        {
            return Ok(GatewayStatus::running(port));
        }
        guard.remove(&port)
    };
    if let Some(runtime) = previous_runtime {
        log::info!("Reloading MCP Gateway port={port} because it was assigned to another channel");
        runtime.cancellation_token.cancel();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| AppError::internal(format!("Failed to bind MCP gateway: {error}")))?;
    let cancellation_token = CancellationToken::new();
    let router = gateway_router(
        &cancellation_token,
        Some(app_handle),
        state.interactions.clone(),
        state.next_interaction_id.clone(),
        channel_scope.clone(),
    );
    let shutdown_token = cancellation_token.clone();
    let runtime_token = cancellation_token.clone();
    let runtimes = state.runtimes.clone();
    let generation = state
        .next_runtime_generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);

    {
        let mut guard = state
            .runtimes
            .lock()
            .map_err(|error| AppError::internal(error.to_string()))?;
        guard.insert(
            port,
            GatewayRuntime {
                cancellation_token,
                channel_scope,
                generation,
            },
        );
    }

    tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown_token.cancelled_owned().await })
            .await;
        let stopped_unexpectedly = !runtime_token.is_cancelled();
        if let Ok(mut guard) = runtimes.lock() {
            if guard
                .get(&port)
                .is_some_and(|runtime| runtime.generation == generation)
            {
                guard.remove(&port);
            }
        }
        if let Err(error) = result {
            log::error!("MCP gateway stopped unexpectedly: {error}");
        } else if stopped_unexpectedly {
            log::warn!("MCP gateway port={port} stopped without a shutdown request");
        }
    });

    Ok(GatewayStatus::running(port))
}

/// Stop the MCP gateway and close all active MCP sessions.
#[tauri::command]
pub fn gateway_stop(state: State<'_, GatewayState>) -> Result<GatewayStatus, AppError> {
    let mut guard = state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    let port = guard.keys().next().copied().unwrap_or(DEFAULT_PORT);
    for (_, runtime) in guard.drain() {
        runtime.cancellation_token.cancel();
    }
    Ok(GatewayStatus::stopped(port))
}

/// Stop one SDK channel gateway without interrupting other boards.
#[tauri::command]
pub fn gateway_stop_port(
    port: u16,
    state: State<'_, GatewayState>,
) -> Result<GatewayStatus, AppError> {
    let mut guard = state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    if let Some(runtime) = guard.remove(&port) {
        runtime.cancellation_token.cancel();
    }
    Ok(GatewayStatus::stopped(port))
}

/// Inspect only the selected saved channel; an absent listener has no usable address.
#[tauri::command]
pub async fn gateway_channel_status(
    project_id: i64,
    device_id: i64,
    state: State<'_, GatewayState>,
) -> Result<GatewayChannelStatus, AppError> {
    connection_status::inspect(state.inner(), project_id, device_id).await
}

/// Return the current gateway endpoint without changing its lifecycle.
#[tauri::command]
pub fn gateway_status(state: State<'_, GatewayState>) -> Result<GatewayStatus, AppError> {
    let guard = state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(match guard.keys().next().copied() {
        Some(port) => GatewayStatus::running(port),
        None => GatewayStatus::stopped(DEFAULT_PORT),
    })
}

/// Return recent AI-to-hardware tool interactions, newest first.
#[tauri::command]
pub fn gateway_list_interactions(
    state: State<'_, GatewayState>,
    limit: Option<usize>,
) -> Result<Vec<GatewayInteraction>, AppError> {
    let guard = state
        .interactions
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(guard
        .iter()
        .rev()
        .take(limit.unwrap_or(100).clamp(1, 200))
        .cloned()
        .collect())
}

/// Clear the in-memory AI interaction timeline.
#[tauri::command]
pub fn gateway_clear_interactions(state: State<'_, GatewayState>) -> Result<(), AppError> {
    state
        .interactions
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?
        .clear();
    Ok(())
}

/// Return host addresses with explicit direct-connect and tunnel semantics.
#[tauri::command]
pub fn gateway_network_info() -> GatewayNetworkInfo {
    let local_ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "未检测到".to_owned());
    let host_name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "LYNX Host".to_owned());
    GatewayNetworkInfo {
        host_name,
        local_ip,
        loopback: "127.0.0.1".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use rmcp::{
        model::{CallToolRequestParams, ClientInfo},
        transport::StreamableHttpClientTransport,
        ServiceExt,
    };

    use super::*;

    #[test]
    fn mainline_tasks_are_persisted_before_usb_work_and_can_finish_cancelled() {
        let database_path = std::env::temp_dir().join(format!(
            "lynx-mainline-task-test-{:032x}.db",
            rand::random::<u128>()
        ));
        let state = crate::workbench::WorkbenchState::open(&database_path).unwrap();
        insert_mainline_task(&state, "mainline-test", 2, 5, 1024).unwrap();

        let running = {
            let db = state.db.lock().unwrap();
            crate::workbench::storage::task_by_id(&db, "mainline-test")
                .unwrap()
                .unwrap()
        };
        assert_eq!(running.kind, "mainline_boot");
        assert_eq!(running.device_id, Some(2));
        assert_eq!(running.status, "running");
        assert_eq!(running.phase, "queued");

        finish_mainline_task(&state, "mainline-test", "cancelled", "TASK_CANCELLED:test");
        let cancelled = {
            let db = state.db.lock().unwrap();
            crate::workbench::storage::task_by_id(&db, "mainline-test")
                .unwrap()
                .unwrap()
        };
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.error, "TASK_CANCELLED:test");
        drop(state);
        let _ = std::fs::remove_file(database_path);
    }

    #[test]
    fn mainline_cancellation_is_checked_between_bounded_chunks() {
        let cancellation = AtomicBool::new(false);
        assert!(ensure_mainline_not_cancelled(&cancellation).is_ok());
        cancellation.store(true, Ordering::Relaxed);
        assert_eq!(
            ensure_mainline_not_cancelled(&cancellation).unwrap_err(),
            "TASK_CANCELLED:mainline FEL boot cancelled by operator"
        );
        assert_eq!(MAINLINE_OPEN_TIMEOUT, Duration::from_secs(20));
        assert_eq!(MAINLINE_PROGRESS_TIMEOUT, Duration::from_secs(30));
        assert_eq!(MAINLINE_WORKER_EXIT_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn stable_location_resolves_one_current_endpoint_without_persisting_it() {
        let devices = vec![
            libefex::ScannedDevice {
                bus: 0,
                port: 4,
                vid: 0x1f3a,
                pid: 0xefe8,
                location_path: Some("pciroot(0)#usbroot(0)#usb(9)".to_owned()),
                device_path: Some("legacy-node".to_owned()),
            },
            libefex::ScannedDevice {
                bus: 0,
                port: 10,
                vid: 0x1f3a,
                pid: 0xefe8,
                location_path: Some("PCIROOT(0)#USBROOT(0)#USB(10)#USB(2)".to_owned()),
                device_path: Some("current-node".to_owned()),
            },
        ];

        let endpoint =
            resolve_current_allwinner_endpoint("pciroot(0)#usbroot(0)#usb(10)#usb(2)", &devices)
                .expect("stable location must resolve the current enumeration");
        assert_eq!(endpoint.bus, 0);
        assert_eq!(endpoint.port, 10);

        let duplicate = vec![devices[1].clone(), devices[1].clone()];
        assert_eq!(
            resolve_current_allwinner_endpoint(
                "pciroot(0)#usbroot(0)#usb(10)#usb(2)",
                &duplicate,
            )
            .unwrap_err(),
            "ALLWINNER_FEL_LOCATION_AMBIGUOUS:physical location resolved to multiple current endpoints"
        );
        assert_eq!(
            resolve_current_allwinner_endpoint("pciroot(0)#usbroot(0)#usb(11)", &devices)
                .unwrap_err(),
            "ALLWINNER_FEL_LOCATION_OFFLINE:bound physical location is not currently enumerated"
        );
    }

    #[test]
    fn opening_timeout_keeps_primary_error_when_cleanup_also_times_out() {
        assert_eq!(
            mainline_error_with_cleanup(
                "ALLWINNER_FEL_OPEN_TIMEOUT:20000ms:location=physical",
                Err("MAINLINE_WORKER_KILL_TIMEOUT".to_owned()),
            ),
            "ALLWINNER_FEL_OPEN_TIMEOUT:20000ms:location=physical:cleanup=MAINLINE_WORKER_KILL_TIMEOUT"
        );
    }

    #[test]
    fn blocked_mainline_helper_is_force_terminated_within_the_exit_budget() {
        #[cfg(unix)]
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 60"])
            .spawn()
            .expect("spawn simulated blocked USB helper");
        #[cfg(windows)]
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .spawn()
            .expect("spawn simulated blocked USB helper");

        let job = crate::process::ProcessJob::attach(&child).expect("scope helper process");
        let started = Instant::now();
        terminate_mainline_worker(&job, &mut child).expect("force terminate blocked helper");

        assert!(started.elapsed() < MAINLINE_WORKER_EXIT_TIMEOUT);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn mainline_worker_output_is_line_bounded_and_preserves_following_messages() {
        let oversized = "x".repeat(MAINLINE_OUTPUT_LINE_LIMIT + 4096);
        let input = format!("{oversized}\n{{\"phase\":\"device_opened\"}}\n");
        let (sender, receiver) = mpsc::channel();
        spawn_mainline_reader(std::io::Cursor::new(input.into_bytes()), "stdout", sender);

        let first = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let second = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(first.0, "stdout");
        assert!(first.1.len() <= MAINLINE_OUTPUT_LINE_LIMIT + "...[truncated]".len());
        assert!(first.1.ends_with("...[truncated]"));
        assert_eq!(second.1, r#"{"phase":"device_opened"}"#);
    }

    #[test]
    fn mainline_worker_plan_uses_unique_private_files() {
        let artifact_path = std::env::temp_dir().join(format!(
            "lynx-mainline-worker-artifact-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::write(&artifact_path, b"artifact").unwrap();
        let artifact = ValidatedMainlineArtifact {
            role: MainlineArtifactRole::Bootloader,
            file_path: artifact_path.to_string_lossy().into_owned(),
            load_address: 0x4000_0000,
            sha256: hex::encode(Sha256::digest(b"artifact")),
            size_bytes: 8,
        };
        let first =
            MainlineWorkerFiles::create("same-task", std::slice::from_ref(&artifact), 0x4000_0000)
                .unwrap();
        let second = MainlineWorkerFiles::create("same-task", &[artifact], 0x4000_0000).unwrap();

        assert_ne!(first.plan, second.plan);
        assert!(first.plan.exists());
        assert!(second.plan.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first.plan).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let first_path = first.plan.clone();
        let second_path = second.plan.clone();
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
        let _ = std::fs::remove_file(artifact_path);
    }

    #[cfg(unix)]
    #[test]
    fn opening_watchdog_terminates_the_entire_helper_process_group() {
        let nonce = rand::random::<u128>();
        let pid_path = std::env::temp_dir().join(format!("lynx-blocked-child-{nonce:032x}.pid"));

        let database_path = std::env::temp_dir().join(format!("lynx-watchdog-{nonce:032x}.db"));
        let state = crate::workbench::WorkbenchState::open(&database_path).unwrap();
        let artifact_path = std::env::temp_dir().join(format!("lynx-watchdog-{nonce:032x}.bin"));
        std::fs::write(&artifact_path, b"artifact").unwrap();
        let artifacts = [ValidatedMainlineArtifact {
            role: MainlineArtifactRole::Bootloader,
            file_path: artifact_path.to_string_lossy().into_owned(),
            load_address: 0x4000_0000,
            sha256: hex::encode(Sha256::digest(b"artifact")),
            size_bytes: 8,
        }];
        insert_mainline_task(&state, "watchdog-test", 1, 1, 8).unwrap();
        let files = MainlineWorkerFiles::create("watchdog-test", &artifacts, 0x4000_0000).unwrap();
        // Execute the existing interpreter directly. On macOS, first execution
        // of a new temporary script can exceed the opening watchdog before the
        // script has created its child, testing OS launch latency instead of
        // process-group cleanup. The production supervision path is shared.
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "sleep 60 &\necho $! > \"$1\"\nwait\n",
                "lynx-watchdog-test",
            ])
            .arg(&pid_path);

        let started = Instant::now();
        let error = supervise_mainline_worker(
            None,
            command,
            &files,
            "pciroot(0)#usbroot(0)#usb(2)",
            8,
            &AtomicBool::new(false),
            &state,
            "watchdog-test",
            Duration::from_millis(150),
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert!(error.starts_with("ALLWINNER_FEL_OPEN_TIMEOUT:150ms:"));
        assert!(started.elapsed() < Duration::from_secs(2));

        let child_pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .trim()
            .parse::<libc::pid_t>()
            .unwrap();
        let child_gone = (0..50).any(|_| {
            let result = unsafe { libc::kill(child_pid, 0) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                true
            } else {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(child_gone, "watchdog left a helper subprocess running");

        drop(state);
        for path in [pid_path, database_path, artifact_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn libefex_bulk_transport_rejects_timeout_and_zero_byte_progress() {
        let source = include_str!("../libs/libefex/src/usb/usb_layer_libusb.c");
        assert_eq!(source.matches("r == LIBUSB_ERROR_TIMEOUT").count(), 2);
        assert_eq!(source.matches("r != 0 || bytes <= 0").count(), 2);
        assert!(source.contains("return EFEX_ERR_USB_TIMEOUT;"));
        assert!(source.contains("libusb_get_port_numbers"));
        assert!(source.contains("format_libusb_location"));
        assert!(source.contains("libusb_scan_device_by_location"));
        assert!(source.contains("Preserve the complete physical hub path"));
        assert!(source.contains("Compatibility with LYNX <=0.6.10"));
        assert!(source.contains("if (matched)"));

        let windows_source = include_str!("../libs/libefex/src/usb/usb_layer_winusb.c");
        assert!(windows_source.contains("FILE_FLAG_OVERLAPPED"));
        assert!(windows_source.contains("ERROR_IO_PENDING"));
        assert!(windows_source.contains("WaitForSingleObject"));
        assert!(windows_source.contains("CancelIoEx"));
        assert!(windows_source.contains("WINUSB_CANCEL_GRACE_MS"));
        assert!(windows_source.contains("SPDRP_LOCATION_PATHS"));
        assert!(windows_source.contains("winusb_scan_device_by_location"));
        assert!(windows_source.contains("_stricmp(candidate, location_path) == 0"));
        assert!(!windows_source.contains("port = (uint8_t)(index + 1)"));
        assert!(!windows_source.contains("target_index = (ULONG)(port - 1)"));
        assert!(!windows_source.contains("transferred, TRUE"));
        assert!(windows_source.contains("return EFEX_ERR_USB_TIMEOUT;"));
    }

    #[test]
    fn blocks_large_allwinner_images_on_full_speed_usb() {
        let error = ensure_reliable_flash_link(
            "allwinner",
            "openixcli",
            "3:1",
            "full",
            506 * 1024 * 1024,
            false,
        )
        .expect_err("large Allwinner image must require a reliable high-speed link");
        assert!(error.starts_with("USB_LINK_TOO_SLOW:"));
        assert!(error.contains("usbSpeed=high"));

        assert!(ensure_reliable_flash_link(
            "allwinner",
            "openixcli",
            "3:1",
            "high",
            506 * 1024 * 1024,
            false,
        )
        .is_ok());
        assert!(ensure_reliable_flash_link(
            "allwinner",
            "openixcli",
            "3:1",
            "full",
            32 * 1024 * 1024,
            false,
        )
        .is_ok());
        assert!(ensure_reliable_flash_link(
            "allwinner",
            "openixcli",
            "3:1",
            "full",
            506 * 1024 * 1024,
            true,
        )
        .is_ok());
    }

    #[test]
    fn rockchip_fixed_recipe_does_not_open_generic_shell_or_serial_policy() {
        assert!(validate_ai_shell_command("reboot loader", Some("rockchip")).is_err());
        assert!(validate_ai_serial_command("reboot loader\r\n", Some("rockchip")).is_err());
        let request: StartFlashRequest = serde_json::from_value(json!({
            "firmwarePath":"C:\\固件\\update.img", "confirmDestructive":true,
            "rockchipEnterLoader":{"method":"serial_shell_reboot_loader", "handle":"serial-1"}
        }))
        .unwrap();
        assert!(request.rockchip_enter_loader.is_some());
        assert!(request.allwinner_enter_fel.is_none());
    }

    #[test]
    fn blocks_unsafe_vendor_boot_transitions_but_allows_normal_diagnostics() {
        for command in [
            "echo platform:syscon-reboot-mode > /sys/bus/platform/drivers/syscon/unbind",
            "devmem 0xff000000 32 1",
            "reboot bootloader",
            "echo maskrom > /sys/reboot-mode",
            "reboot efex",
            "reboot fastboot",
            "reboot brom",
            "reboot dfu",
        ] {
            assert!(validate_ai_shell_command(command, Some("allwinner"))
                .unwrap_err()
                .starts_with("UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:allwinner"));
        }
        assert!(validate_ai_shell_command("dmesg | tail -100", Some("allwinner")).is_ok());
        assert!(validate_ai_shell_command("mount | grep ubi", Some("allwinner")).is_ok());
    }

    #[test]
    fn k230_serial_upgrade_is_scoped_and_uses_the_vendor_sequence() {
        assert!(validate_k230_upgrade_scope("kendryte", "k230").is_ok());
        for (vendor, flasher) in [
            ("allwinner", "openix"),
            ("rockchip", "openrock"),
            ("kendryte", "openix"),
        ] {
            assert!(validate_k230_upgrade_scope(vendor, flasher)
                .unwrap_err()
                .starts_with("K230_UPGRADE_MODE_UNSUPPORTED:"));
        }
        assert_eq!(K230_UPGRADE_INTERRUPT, &[0x03]);
        assert_eq!(K230_UPGRADE_PROMPT, b"msh />");
        assert_eq!(K230_UPGRADE_COMMAND, b"reboot_to_upgrade\r\n");
        assert_eq!(
            K230_UPGRADE_COMMAND,
            &[
                0x72, 0x65, 0x62, 0x6f, 0x6f, 0x74, 0x5f, 0x74, 0x6f, 0x5f, 0x75, 0x70, 0x67, 0x72,
                0x61, 0x64, 0x65, 0x0d, 0x0a,
            ]
        );
        assert!(!K230_UPGRADE_COMMAND.contains(&0xff));
        assert_eq!(K230_STALE_DRAIN_PASSES, 4);
    }

    #[test]
    fn allwinner_fel_recipe_is_scoped_and_uses_only_fixed_commands() {
        assert!(validate_allwinner_fel_scope("allwinner", "openixcli").is_ok());
        for (vendor, flasher) in [
            ("allwinner", "openix"),
            ("rockchip", "openixcli"),
            ("kendryte", "k230"),
        ] {
            assert!(validate_allwinner_fel_scope(vendor, flasher)
                .unwrap_err()
                .starts_with("ALLWINNER_FEL_UNSUPPORTED:"));
        }
        assert_eq!(ALLWINNER_ADB_EFEX_COMMAND, "reboot efex");
        assert_eq!(ALLWINNER_FEL_INTERRUPT, &[0x03]);
        assert_eq!(ALLWINNER_UBOOT_PROMPT, b"=>");
        assert_eq!(ALLWINNER_UBOOT_EFEX_COMMAND, b"efex\r\n");
        assert_eq!(ALLWINNER_SERIAL_SHELL_EFEX_COMMAND, b"reboot efex\r\n");
        assert_eq!(ALLWINNER_STALE_DRAIN_PASSES, 4);
        assert!(ALLWINNER_EFEX_TX_SETTLE >= Duration::from_millis(250));
        assert!(contains_allwinner_uboot_prompt(b"U-Boot ready\r\n=> "));
        assert!(contains_allwinner_uboot_prompt(b"=>"));
        assert!(!contains_allwinner_uboot_prompt(b"key=>value"));
        assert!(!contains_allwinner_uboot_prompt(b"log says => maybe"));
        assert!(contains_serial_shell_prompt(b"root@TinaLinux:/# ", "#"));
        assert!(contains_serial_shell_prompt(
            b"aLinux:/# \r\nroot@TinaLinux:/# \r\nroot@Tin",
            "#"
        ));
        assert!(!contains_serial_shell_prompt(b"kernel # warning\r\n", "#"));
        assert!(!contains_serial_shell_prompt(
            b"kernel warning #\r\nnext",
            "#"
        ));

        // The generic free-form tool must remain blocked. Only the dedicated,
        // board-scoped recipe may use the compiled-in exact command.
        assert!(
            validate_ai_shell_command(ALLWINNER_ADB_EFEX_COMMAND, Some("allwinner"))
                .unwrap_err()
                .starts_with("UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:allwinner")
        );
        assert!(validate_ai_serial_command("efex\r\n", Some("allwinner"))
            .unwrap_err()
            .starts_with("UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:allwinner"));
        assert!(validate_ai_serial_command("help; efex\r\n", None)
            .unwrap_err()
            .starts_with("UNSAFE_BOOT_MODE_TRANSITION_BLOCKED:unscoped serial"));
        assert!(validate_ai_serial_command("help\r\n", Some("allwinner")).is_ok());
    }

    #[test]
    fn mainline_plan_requires_hash_bounded_addresses_and_declared_entry() {
        let path = std::env::temp_dir().join(format!("lynx-mainline-plan-{}", std::process::id()));
        let spl_path =
            std::env::temp_dir().join(format!("lynx-mainline-spl-plan-{}", std::process::id()));
        std::fs::write(&path, b"mainline-image").unwrap();
        let digest = hex::encode(Sha256::digest(b"mainline-image"));
        let mut spl = vec![0_u8; 0x6000];
        spl[4..12].copy_from_slice(b"eGON.BT0");
        spl[0x0c..0x10].copy_from_slice(&0x5f0a_6c39_u32.to_le_bytes());
        let spl_length = spl.len() as u32;
        spl[0x10..0x14].copy_from_slice(&spl_length.to_le_bytes());
        spl[0x14..0x17].copy_from_slice(b"SPL");
        spl[0x17] = 2;
        let checksum = spl.as_chunks::<4>().0.iter().fold(0_u32, |sum, word| {
            sum.wrapping_add(u32::from_le_bytes(*word))
        });
        spl[0x0c..0x10].copy_from_slice(&checksum.to_le_bytes());
        std::fs::write(&spl_path, &spl).unwrap();
        let spl_digest = hex::encode(Sha256::digest(&spl));
        let request = |sha256: String, entry_address: u32, confirm_destructive: bool| {
            AllwinnerMainlineBootRequest {
                artifacts: vec![
                    MainlineArtifactRequest {
                        role: MainlineArtifactRole::Spl,
                        file_path: spl_path.to_string_lossy().into_owned(),
                        load_address: R528_MAINLINE_SPL_ADDRESS,
                        sha256: spl_digest.clone(),
                    },
                    MainlineArtifactRequest {
                        role: MainlineArtifactRole::Bootloader,
                        file_path: path.to_string_lossy().into_owned(),
                        load_address: 0x4a00_0000,
                        sha256,
                    },
                ],
                entry_address,
                confirm_destructive,
            }
        };

        let (artifacts, entry) =
            validate_mainline_plan(request(digest.clone(), 0x4a00_0000, true)).unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[1].sha256, digest);
        assert_eq!(entry, 0x4a00_0000);
        assert!(
            validate_mainline_plan(request(digest.clone(), 0x4a00_0000, false))
                .unwrap_err()
                .starts_with("CONFIRMATION_REQUIRED:")
        );
        assert_eq!(
            validate_mainline_plan(request("0".repeat(64), 0x4a00_0000, true)).unwrap_err(),
            "MAINLINE_ARTIFACT_HASH_MISMATCH"
        );
        assert_eq!(
            validate_mainline_plan(request(digest, 0x4a10_0000, true)).unwrap_err(),
            "MAINLINE_ENTRY_MUST_BE_BOOTLOADER"
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(spl_path);
    }

    #[test]
    fn mainline_spl_rejects_vendor_boot0_and_corruption() {
        let mut spl = vec![0_u8; 0x6000];
        spl[4..12].copy_from_slice(b"eGON.BT0");
        spl[0x0c..0x10].copy_from_slice(&0x5f0a_6c39_u32.to_le_bytes());
        spl[0x10..0x14].copy_from_slice(&0x6000_u32.to_le_bytes());
        spl[0x14..0x17].copy_from_slice(b"BT0");
        spl[0x17] = 2;
        assert!(validate_r528_mainline_spl(&spl)
            .unwrap_err()
            .starts_with("MAINLINE_SPL_SIGNATURE_REQUIRED:"));

        spl[0x14..0x17].copy_from_slice(b"SPL");
        assert_eq!(
            validate_r528_mainline_spl(&spl).unwrap_err(),
            "MAINLINE_SPL_CHECKSUM_INVALID"
        );
    }

    #[test]
    fn k230_upgrade_invalidates_every_handle_for_the_reenumerating_port() {
        let gateway = LynxMcp::new(
            None,
            Arc::new(Mutex::new(VecDeque::new())),
            Arc::new(AtomicU64::new(0)),
            None,
        );
        {
            let mut ports = gateway.serial.handle_ports.lock().unwrap();
            ports.insert("serial-1".to_owned(), "COM9".to_owned());
            ports.insert("serial-2".to_owned(), "COM9".to_owned());
            ports.insert("serial-3".to_owned(), "COM10".to_owned());
        }
        {
            let mut bauds = gateway.serial.handle_bauds.lock().unwrap();
            bauds.insert("serial-1".to_owned(), 115_200);
            bauds.insert("serial-2".to_owned(), 115_200);
            bauds.insert("serial-3".to_owned(), 115_200);
        }

        assert_eq!(
            gateway
                .release_serial_handles_for_port("serial-1", "COM9")
                .unwrap(),
            2
        );
        assert!(gateway.serial_binding("serial-1").is_err());
        assert!(gateway.serial_binding("serial-2").is_err());
        assert_eq!(
            gateway.serial_binding("serial-3").unwrap(),
            ("COM10".to_owned(), 115_200)
        );
    }

    #[test]
    fn interactions_keep_device_keys_separate_for_multi_board_routing() {
        let interactions = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
        let gateway = LynxMcp::new(
            None,
            interactions.clone(),
            Arc::new(AtomicU64::new(0)),
            None,
        );
        for index in 1..=4 {
            gateway.record_device_interaction(
                "lynx_serial_write",
                "串口",
                format!("board-{index}"),
                format!("serial:/dev/ttySIM{index}"),
            );
        }
        let items = interactions.lock().expect("interaction lock");
        assert_eq!(items.len(), 4);
        for index in 1..=4 {
            assert_eq!(
                items[index - 1].device_key,
                format!("serial:/dev/ttySIM{index}")
            );
            assert_eq!(items[index - 1].summary, format!("board-{index}"));
        }
    }

    #[test]
    fn serial_handles_survive_mcp_session_recreation() {
        let serial = SerialSessions::default();
        serial
            .handle_ports
            .lock()
            .unwrap()
            .insert("serial-1".to_owned(), "/dev/ttySIM".to_owned());
        serial
            .handle_bauds
            .lock()
            .unwrap()
            .insert("serial-1".to_owned(), 115_200);
        let gateway = LynxMcp::with_serial(
            None,
            Arc::new(Mutex::new(VecDeque::new())),
            Arc::new(AtomicU64::new(0)),
            None,
            serial,
        );
        assert_eq!(
            gateway.serial_binding("serial-1").unwrap(),
            ("/dev/ttySIM".to_owned(), 115_200)
        );
    }

    #[test]
    fn serial_requests_accept_camel_case_parameters() {
        let open: OpenSerialRequest = serde_json::from_value(json!({
            "port": "/dev/ttySIM",
            "baudRate": 115200
        }))
        .unwrap();
        assert_eq!(open.baud_rate, 115_200);

        let read: ReadSerialRequest = serde_json::from_value(json!({
            "handle": "serial-1",
            "maxBytes": 1,
            "timeoutMs": 5000
        }))
        .unwrap();
        assert_eq!(read.max_bytes, Some(1));
        assert_eq!(read.timeout_ms, Some(5_000));

        let write: WriteSerialRequest = serde_json::from_value(json!({
            "handle": "serial-1",
            "data": "help",
            "appendNewline": true
        }))
        .unwrap();
        assert_eq!(write.append_newline, Some(true));
    }

    #[test]
    fn start_flash_accepts_one_guard_allwinner_transition_contract() {
        let request: StartFlashRequest = serde_json::from_value(json!({
            "firmwarePath": "/cache/mainline.raw",
            "confirmDestructive": true,
            "options": {
                "vendorOptions": {
                    "raw": true,
                    "bootstrapFirmwarePath": "/cache/loader.img"
                }
            },
            "allwinnerEnterFel": {
                "method": "uboot_serial_efex",
                "handle": "serial-7",
                "preserveSerialMonitor": true
            }
        }))
        .unwrap();
        let transition = request.allwinner_enter_fel.expect("transition recipe");
        assert!(matches!(
            transition.method,
            AllwinnerFelMethod::UbootSerialEfex
        ));
        assert_eq!(transition.handle.as_deref(), Some("serial-7"));
        assert!(transition.preserve_serial_monitor);

        let request: StartFlashRequest = serde_json::from_value(json!({
            "firmwarePath": "/cache/vendor.img",
            "confirmDestructive": true,
            "allwinnerEnterFel": {
                "method": "serial_shell_reboot_efex",
                "handle": "serial-8"
            }
        }))
        .unwrap();
        let transition = request
            .allwinner_enter_fel
            .expect("serial shell transition");
        assert!(matches!(
            transition.method,
            AllwinnerFelMethod::SerialShellRebootEfex
        ));
        assert_eq!(transition.handle.as_deref(), Some("serial-8"));
        assert!(!transition.preserve_serial_monitor);
    }

    #[tokio::test]
    async fn four_channel_scopes_are_isolated() {
        for index in 1..=4 {
            let interactions = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
            let gateway = LynxMcp::new(
                None,
                interactions.clone(),
                Arc::new(AtomicU64::new(0)),
                Some(ChannelScope {
                    project_id: index + 100,
                    project_name: format!("SDK {index}"),
                    mcp_server_name: format!("lynx-sdk-{index}"),
                    device_id: index,
                    chip_vendor: "allwinner".to_owned(),
                    flasher_kind: "openixcli".to_owned(),
                    flasher_path: String::new(),
                    serial_port: format!("/dev/ttySIM{index}"),
                    usb_binding: format!("9:{index}"),
                    adb_serial: format!("sim-adb-{index}"),
                    camera_binding: String::new(),
                }),
            );
            assert!(gateway
                .ensure_serial_scope(&format!("/dev/ttySIM{index}"))
                .is_ok());
            assert!(gateway
                .ensure_serial_scope(&format!("/dev/ttySIM{}", (index % 4) + 1))
                .is_err());
            gateway.record_interaction("lynx_workbench_status", "连接", "scope check");
            {
                let items = interactions.lock().expect("interaction lock");
                let interaction = items.back().expect("scoped interaction");
                assert_eq!(
                    interaction.device_key,
                    format!("project:{}:device:{index}", index + 100)
                );
                assert_eq!(interaction.project_id, Some(index + 100));
                assert_eq!(interaction.device_id, Some(index));
            }
            let status: Value =
                serde_json::from_str(&gateway.workbench_status().await).expect("status json");
            assert_eq!(status["deviceId"], index);
            assert_eq!(status["projectId"], index + 100);
            assert_eq!(status["mcpName"], format!("lynx-sdk-{index}"));
            assert!(status["capabilityManifest"]["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "lynx_workbench_list_tasks"));
        }
    }

    #[test]
    fn gateway_watchdog_keeps_sessions_when_live_interfaces_reenumerate() {
        let scope = |project_id, serial_port: &str, usb_binding: &str| ChannelScope {
            project_id,
            project_name: "SDK".to_owned(),
            mcp_server_name: "lynx-sdk".to_owned(),
            device_id: 22,
            chip_vendor: "allwinner".to_owned(),
            flasher_kind: "openixcli".to_owned(),
            flasher_path: String::new(),
            serial_port: serial_port.to_owned(),
            usb_binding: usb_binding.to_owned(),
            adb_serial: String::new(),
            camera_binding: String::new(),
        };
        let current = Some(scope(11, "COM17", "location:libusb:2:2"));
        let reenumerated = Some(scope(11, "COM19", "location:libusb:2:3.2"));
        let other_channel = Some(scope(12, "COM19", "location:libusb:2:3.2"));

        assert!(same_gateway_route(&current, &reenumerated));
        assert!(!same_gateway_route(&current, &other_channel));
        assert!(same_gateway_route(&None, &None));
        assert!(!same_gateway_route(&current, &None));
    }

    #[test]
    fn board_channel_accepts_device_scoped_flash_tasks() {
        let gateway = LynxMcp::new(
            None,
            Arc::new(Mutex::new(VecDeque::new())),
            Arc::new(AtomicU64::new(0)),
            Some(ChannelScope {
                project_id: 11,
                project_name: "SDK".to_owned(),
                mcp_server_name: "lynx-sdk".to_owned(),
                device_id: 22,
                chip_vendor: "allwinner".to_owned(),
                flasher_kind: "openixcli".to_owned(),
                flasher_path: String::new(),
                serial_port: "/dev/ttySIM".to_owned(),
                usb_binding: "1:2".to_owned(),
                adb_serial: String::new(),
                camera_binding: String::new(),
            }),
        );
        let task = crate::workbench::types::TaskHistory {
            task_id: "flash-1".to_owned(),
            kind: "flash".to_owned(),
            project_id: None,
            device_id: Some(22),
            status: "running".to_owned(),
            started_at: 1,
            finished_at: None,
            exit_code: None,
            error: String::new(),
            phase: "write".to_owned(),
            progress_pct: Some(50.0),
            details: Some(json!({"usbSpeed": "high"})),
        };
        assert!(gateway.ensure_task_scope(&task).is_ok());
        let value = task_status_value(&task);
        assert_eq!(value["state"], "running");
        assert_eq!(value["phase"], "write");
        assert_eq!(value["usbSpeed"], "high");
    }

    #[test]
    fn mainline_flash_console_rejects_serial_output() {
        assert!(mainline_flash_log_stream_allowed("fel"));
        assert!(mainline_flash_log_stream_allowed("stderr"));
        assert!(!mainline_flash_log_stream_allowed("uart"));
        assert!(!mainline_flash_log_stream_allowed("serial"));
    }

    #[tokio::test]
    async fn streamable_http_client_can_discover_and_call_gateway_tools() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cancellation_token = CancellationToken::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test gateway");
        let address = listener.local_addr().expect("read test gateway address");
        let router = gateway_router(
            &cancellation_token,
            None,
            Arc::new(Mutex::new(VecDeque::with_capacity(200))),
            Arc::new(AtomicU64::new(0)),
            None,
        );
        let shutdown_token = cancellation_token.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown_token.cancelled_owned().await })
                .await
                .expect("serve test gateway");
        });

        let client = ClientInfo::default()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "http://{address}/mcp"
            )))
            .await
            .expect("initialize MCP client");

        let tools = client.list_all_tools().await.expect("list MCP tools");
        let tool_names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            tool_names,
            [
                "lynx_adb_list",
                "lynx_adb_shell",
                "lynx_allwinner_boot_mainline",
                "lynx_allwinner_enter_fel",
                "lynx_camera_capture",
                "lynx_download_firmware",
                "lynx_flash_capabilities",
                "lynx_inspect_firmware",
                "lynx_k230_enter_upgrade_mode",
                "lynx_power_capabilities",
                "lynx_power_control",
                "lynx_power_status",
                "lynx_rockchip_enter_loader",
                "lynx_scan_flash_devices",
                "lynx_serial_close",
                "lynx_serial_list",
                "lynx_serial_open",
                "lynx_serial_read",
                "lynx_serial_write",
                "lynx_skills_get",
                "lynx_skills_list_systems",
                "lynx_skills_read",
                "lynx_skills_search",
                "lynx_start_flash",
                "lynx_supervisor_list_automatic_tasks",
                "lynx_supervisor_trigger_automatic_task",
                "lynx_workbench_cancel_task",
                "lynx_workbench_list_tasks",
                "lynx_workbench_status",
                "lynx_workbench_task_status"
            ]
        );
        let list_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_workbench_list_tasks")
            .unwrap();
        assert!(list_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .is_none_or(|required| required.is_empty()));
        let response = client
            .call_tool(CallToolRequestParams::new("lynx_workbench_list_tasks"))
            .await
            .expect("unscoped history call");
        assert_eq!(response.is_error, Some(true));
        assert!(serde_json::to_string(&response)
            .unwrap()
            .contains("CHANNEL_SCOPE"));
        let loader_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_rockchip_enter_loader")
            .unwrap();
        let schema = serde_json::to_value(&loader_tool.input_schema).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|field| field == "confirmDestructive"));
        assert!(required.iter().any(|field| field == "method"));
        assert!(schema["properties"].get("command").is_none());
        let response = client
            .call_tool(
                CallToolRequestParams::new("lynx_rockchip_enter_loader").with_arguments(
                    json!({"method":"adb_reboot_loader", "confirmDestructive":true})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .expect("unscoped Loader call");
        assert_eq!(response.is_error, Some(true));
        assert!(serde_json::to_string(&response)
            .unwrap()
            .contains("CHANNEL_SCOPE"));
        let allwinner_fel_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_allwinner_enter_fel")
            .expect("Allwinner FEL tool");
        let required = allwinner_fel_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("Allwinner FEL required fields");
        assert!(required.iter().any(|field| field == "method"));
        assert!(required.iter().any(|field| field == "confirmDestructive"));
        assert!(!allwinner_fel_tool.input_schema.contains_key("command"));
        let mainline_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_allwinner_boot_mainline")
            .expect("Allwinner mainline tool");
        let mainline_required = mainline_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("Allwinner mainline required fields");
        for field in ["artifacts", "entryAddress", "confirmDestructive"] {
            assert!(mainline_required.iter().any(|item| item == field));
        }
        assert!(serde_json::to_string(&mainline_tool.input_schema)
            .expect("mainline schema JSON")
            .contains("\"spl\""));
        assert!(!mainline_tool.input_schema.contains_key("command"));
        assert!(!mainline_tool.input_schema.contains_key("args"));
        let download_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_download_firmware")
            .expect("download tool");
        assert!(download_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|field| field == "remotePath")));
        let flash_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_start_flash")
            .expect("flash tool");
        let required = flash_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("required flash fields");
        assert!(required.iter().any(|field| field == "confirmDestructive"));
        assert!(required.iter().any(|field| field == "firmwarePath"));
        assert!(!required.iter().any(|field| field == "confirmation"));
        let cancel_tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_workbench_cancel_task")
            .expect("cancel task tool");
        let status_required = cancel_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("required flash status fields");
        assert!(status_required.iter().any(|field| field == "taskId"));
        assert!(!status_required.iter().any(|field| field == "task_id"));
        let skills_search = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "lynx_skills_search")
            .expect("skills search tool");
        let skills_required = skills_search
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("skills required fields");
        assert!(skills_required.iter().any(|field| field == "query"));
        assert!(!skills_required.iter().any(|field| field == "system"));
        assert!(skills_search.input_schema["properties"]
            .get("system")
            .is_some());

        for request in [
            CallToolRequestParams::new("lynx_skills_search").with_arguments(
                serde_json::from_value(json!({"query":"T113S3 NAND", "system":"linux"}))
                    .expect("search arguments"),
            ),
            CallToolRequestParams::new("lynx_skills_get").with_arguments(
                serde_json::from_value(json!({"name":"t113s3-mainline-nand"}))
                    .expect("get arguments"),
            ),
            CallToolRequestParams::new("lynx_skills_read").with_arguments(
                serde_json::from_value(json!({"name":"t113s3-mainline-nand", "maxBytes":128}))
                    .expect("read arguments"),
            ),
        ] {
            let response = client.call_tool(request).await.expect("skills MCP call");
            assert_ne!(response.is_error, Some(true));
        }

        let response = client
            .call_tool(CallToolRequestParams::new("lynx_workbench_status"))
            .await
            .expect("call status tool");
        assert_ne!(response.is_error, Some(true));

        let response = client
            .call_tool(CallToolRequestParams::new("lynx_serial_list"))
            .await
            .expect("list serial ports");
        assert_ne!(response.is_error, Some(true));

        let _ = client.cancel().await;
        cancellation_token.cancel();
        server.await.expect("join test gateway");
    }
}

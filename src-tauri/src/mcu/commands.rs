use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{command, Manager, State};

use super::boards::{board_by_id, McuBoardConfig, ProbeBackend};
use super::cube_programmer;
use super::jlink;
use super::openocd;
use super::pyocd;
use super::McuState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McuError {
    pub code: i32,
    pub name: &'static str,
    pub message: String,
}

impl std::fmt::Display for McuError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.name, self.message)
    }
}

impl std::error::Error for McuError {}

impl McuError {
    pub(super) fn tool_not_found(message: impl Into<String>) -> Self {
        Self {
            code: 4101,
            name: "McuToolNotFound",
            message: message.into(),
        }
    }

    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: 4102,
            name: "McuInvalidConfiguration",
            message: message.into(),
        }
    }

    pub(super) fn operation(message: impl Into<String>) -> Self {
        Self {
            code: 4103,
            name: "McuOperationFailed",
            message: message.into(),
        }
    }

    pub(super) fn timeout(message: impl Into<String>) -> Self {
        Self {
            code: 4104,
            name: "McuOperationTimeout",
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequest {
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub available: bool,
    pub executable_path: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StLinkProbe {
    pub index: u32,
    pub serial_number: String,
    pub firmware_version: String,
    pub board_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StLinkConnection {
    pub executable_path: Option<String>,
    pub probe_serial: Option<String>,
    pub probe_index: Option<u32>,
    pub frequency_khz: u32,
    pub connect_mode: ConnectMode,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectMode {
    Normal,
    UnderReset,
    HotPlug,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub connected: bool,
    pub device_name: String,
    pub device_id: String,
    pub voltage: Option<f64>,
    pub core_state: String,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetActionRequest {
    pub connection: StLinkConnection,
    pub action: TargetAction,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetAction {
    ResetRun,
    ResetHalt,
    Run,
    Halt,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashRequest {
    pub connection: StLinkConnection,
    pub firmware_path: String,
    pub external_loader_path: String,
    pub external_address_start: u64,
    pub external_address_size: u64,
    pub bin_start_address: Option<u64>,
    pub post_action: FlashPostAction,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlashPostAction {
    None,
    ResetRun,
    ResetHalt,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McuPowerRequest {
    pub board_id: String,
    pub device_index: u32,
    pub gpio: u8,
    pub active_high: bool,
    pub action: McuPowerAction,
    pub off_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McuPowerAction {
    On,
    Off,
    Cycle,
}

#[derive(Clone, Copy)]
pub(crate) enum McuAuditActor {
    User,
    Mcp,
}

impl McuAuditActor {
    fn identity(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user", "local-user"),
            Self::Mcp => ("ai", "mcu-mcp-gateway"),
        }
    }
}

#[command]
pub fn mcu_ch347_power_action(
    request: McuPowerRequest,
    app: tauri::AppHandle,
    state: State<'_, Arc<crate::ch347::Ch347State>>,
) -> Result<String, McuError> {
    let board = board_by_id(&app, &request.board_id)?;
    validate_power_binding(&board, &request)?;
    if !(4..=7).contains(&request.gpio) {
        return Err(McuError::operation(format!(
            "MCU_POWER_GPIO_INVALID:{}:expected 4..7",
            request.gpio
        )));
    }
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin_scoped(
            "MCU 工作台",
            "继电器电源操作",
            format!("CH347F:{}:GPIO{}", request.device_index, request.gpio),
            "ch347",
        )
        .map_err(McuError::operation)?;

    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "power",
        power_action_name(request.action),
        serde_json::json!({
            "controller": "CH347F",
            "gpio": request.gpio,
            "offTimeMs": request.off_time_ms.unwrap_or(800).clamp(100, 30_000),
        }),
    )?;
    if matches!(request.action, McuPowerAction::Off | McuPowerAction::Cycle) {
        release_board_uart(&app, &board)?;
    }

    crate::ch347::commands::open_core(state.inner().as_ref(), request.device_index)
        .map_err(McuError::operation)?;
    let mask = 1u8 << request.gpio;
    let set_power = |on: bool| {
        let high = on == request.active_high;
        crate::ch347::commands::gpio_set_core(
            state.inner().as_ref(),
            request.device_index,
            mask,
            mask,
            if high { mask } else { 0 },
        )
    };
    let operation = match request.action {
        McuPowerAction::On => set_power(true),
        McuPowerAction::Off => set_power(false),
        McuPowerAction::Cycle => set_power(false).and_then(|()| {
            std::thread::sleep(std::time::Duration::from_millis(
                request.off_time_ms.unwrap_or(800).clamp(100, 30_000),
            ));
            set_power(true)
        }),
    };
    let close = crate::ch347::commands::close_core(state.inner().as_ref(), request.device_index);
    let result = operation
        .map_err(McuError::operation)
        .and_then(|()| close.map_err(McuError::operation))
        .map(|()| {
            match request.action {
                McuPowerAction::On | McuPowerAction::Cycle => "on",
                McuPowerAction::Off => "off",
            }
            .to_owned()
        });
    publish_mcu_result(
        &app,
        &board.id,
        McuAuditActor::User,
        "power",
        power_action_name(request.action),
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_stlink_tool_info(request: ToolRequest) -> Result<ToolInfo, McuError> {
    tauri::async_runtime::spawn_blocking(move || cube_programmer::tool_info(&request))
        .await
        .map_err(|error| McuError::operation(format!("工具检测任务失败：{error}")))?
}

#[command]
pub async fn mcu_stlink_list_probes(
    request: ToolRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<Vec<StLinkProbe>, McuError> {
    crate::hardware_safety::ensure_idle(&app, "MCU 探针枚举").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "探针枚举", "ST-Link")
        .map_err(McuError::operation)?;
    let _guard = state.operation.lock().await;
    tauri::async_runtime::spawn_blocking(move || cube_programmer::list_probes(&request))
        .await
        .map_err(|error| McuError::operation(format!("探针枚举任务失败：{error}")))?
}

#[command]
pub async fn mcu_stlink_target_status(
    request: StLinkConnection,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<TargetStatus, McuError> {
    crate::hardware_safety::ensure_idle(&app, "MCU 状态读取").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "状态读取", "ST-Link")
        .map_err(McuError::operation)?;
    let _guard = state.operation.lock().await;
    tauri::async_runtime::spawn_blocking(move || cube_programmer::target_status(&request))
        .await
        .map_err(|error| McuError::operation(format!("目标状态任务失败：{error}")))?
}

#[command]
pub async fn mcu_stlink_target_action(
    board_id: String,
    request: TargetActionRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let action = request.action;
    let board = board_by_id(&app, &board_id)?;
    validate_stlink_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 目标控制").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "目标控制", "ST-Link")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        serde_json::json!({"backend": "cubeProgrammer"}),
    )?;
    if matches!(action, TargetAction::ResetRun | TargetAction::ResetHalt) {
        release_board_uart(&app, &board)?;
    }
    let _guard = state.operation.lock().await;
    let result =
        tauri::async_runtime::spawn_blocking(move || cube_programmer::target_action(&request))
            .await
            .map_err(|error| McuError::operation(format!("目标控制任务失败：{error}")))
            .and_then(|result| result);
    publish_mcu_result(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_stlink_flash_ospi(
    board_id: String,
    request: FlashRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let board = board_by_id(&app, &board_id)?;
    validate_stlink_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU OSPI 烧写").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "OSPI 烧写", "ST-Link")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "ospi",
        serde_json::json!({
            "backend": "cubeProgrammer", "address": request.bin_start_address,
            "postAction": flash_post_action_name(request.post_action),
        }),
    )?;
    release_board_uart(&app, &board)?;
    let _guard = state.operation.lock().await;
    let result =
        tauri::async_runtime::spawn_blocking(move || cube_programmer::flash_ospi(&request))
            .await
            .map_err(|error| McuError::operation(format!("OSPI 烧写任务失败：{error}")))
            .and_then(|result| result);
    publish_mcu_result(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "ospi",
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_openocd_tool_info(
    request: openocd::OpenOcdToolRequest,
    app: tauri::AppHandle,
) -> Result<openocd::OpenOcdToolInfo, McuError> {
    tauri::async_runtime::spawn_blocking(move || openocd::tool_info(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("OpenOCD 检测任务失败：{error}")))?
}

#[command]
pub async fn mcu_openocd_target_status(
    request: openocd::OpenOcdConnection,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<TargetStatus, McuError> {
    crate::hardware_safety::ensure_idle(&app, "MCU 状态读取").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "状态读取", "OpenOCD")
        .map_err(McuError::operation)?;
    let _guard = state.operation.lock().await;
    tauri::async_runtime::spawn_blocking(move || openocd::target_status(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("OpenOCD 状态任务失败：{error}")))?
}

#[command]
pub async fn mcu_openocd_target_action(
    board_id: String,
    request: openocd::OpenOcdActionRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let action = request.action;
    let board = board_by_id(&app, &board_id)?;
    validate_openocd_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 目标控制").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "目标控制", "OpenOCD")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        serde_json::json!({"backend": "openOcd"}),
    )?;
    if matches!(action, TargetAction::ResetRun | TargetAction::ResetHalt) {
        release_board_uart(&app, &board)?;
    }
    let _guard = state.operation.lock().await;
    let audit_app = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || openocd::target_action(&app, &request))
            .await
            .map_err(|error| McuError::operation(format!("OpenOCD 控制任务失败：{error}")))
            .and_then(|result| result);
    publish_mcu_result(
        &audit_app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_openocd_flash_ospi(
    board_id: String,
    request: openocd::OpenOcdFlashRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let board = board_by_id(&app, &board_id)?;
    validate_openocd_binding(&board, &request.connection)?;
    if board.openocd_flash_bank != request.flash_bank {
        return Err(McuError::invalid("MCU_FLASH_BANK_BINDING_MISMATCH"));
    }
    crate::hardware_safety::ensure_idle(&app, "MCU OSPI 烧写").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "OSPI 烧写", "OpenOCD")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "ospi",
        serde_json::json!({
            "backend": "openOcd", "address": request.bin_start_address,
            "postAction": flash_post_action_name(request.post_action),
        }),
    )?;
    release_board_uart(&app, &board)?;
    let _guard = state.operation.lock().await;
    let audit_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || openocd::flash_ospi(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("OpenOCD OSPI 烧写任务失败：{error}")))
        .and_then(|result| result);
    publish_mcu_result(
        &audit_app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "ospi",
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_pyocd_tool_info(
    request: pyocd::PyOcdToolRequest,
    app: tauri::AppHandle,
) -> Result<pyocd::PyOcdToolInfo, McuError> {
    tauri::async_runtime::spawn_blocking(move || pyocd::tool_info(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("pyOCD 检测任务失败：{error}")))?
}

#[command]
pub async fn mcu_pyocd_target_status(
    request: pyocd::PyOcdConnection,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<TargetStatus, McuError> {
    crate::hardware_safety::ensure_idle(&app, "MCU 状态读取").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "状态读取", "pyOCD")
        .map_err(McuError::operation)?;
    let _guard = state.operation.lock().await;
    tauri::async_runtime::spawn_blocking(move || pyocd::target_status(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("pyOCD 状态任务失败：{error}")))?
}

#[command]
pub async fn mcu_pyocd_target_action(
    board_id: String,
    request: pyocd::PyOcdActionRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let action = request.action;
    let board = board_by_id(&app, &board_id)?;
    validate_pyocd_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 目标控制").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "目标控制", "pyOCD")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        serde_json::json!({"backend": "pyOcd"}),
    )?;
    if matches!(action, TargetAction::ResetRun | TargetAction::ResetHalt) {
        release_board_uart(&app, &board)?;
    }
    let _guard = state.operation.lock().await;
    let audit_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || pyocd::target_action(&app, &request))
        .await
        .map_err(|error| McuError::operation(format!("pyOCD 控制任务失败：{error}")))
        .and_then(|result| result);
    publish_mcu_result(
        &audit_app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_pyocd_flash_internal(
    board_id: String,
    request: pyocd::PyOcdFlashRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let board = board_by_id(&app, &board_id)?;
    validate_pyocd_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 内部 Flash 烧写")
        .map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "内部 Flash 烧写", "pyOCD")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "internal",
        serde_json::json!({
            "backend": "pyOcd", "address": request.bin_start_address,
            "postAction": flash_post_action_name(request.post_action),
        }),
    )?;
    release_board_uart(&app, &board)?;
    let _guard = state.operation.lock().await;
    let audit_app = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || pyocd::flash_internal(&app, &request))
            .await
            .map_err(|error| McuError::operation(format!("pyOCD 内部 Flash 烧写任务失败：{error}")))
            .and_then(|result| result);
    publish_mcu_result(
        &audit_app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "internal",
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_jlink_tool_info(
    request: jlink::JLinkToolRequest,
) -> Result<jlink::JLinkToolInfo, McuError> {
    tauri::async_runtime::spawn_blocking(move || jlink::tool_info(&request))
        .await
        .map_err(|error| McuError::operation(format!("J-Link 检测任务失败：{error}")))?
}

#[command]
pub async fn mcu_jlink_target_status(
    board_id: String,
    request: jlink::JLinkConnection,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<TargetStatus, McuError> {
    let board = board_by_id(&app, &board_id)?;
    validate_jlink_binding(&board, &request)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 状态读取").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "状态读取", "J-Link")
        .map_err(McuError::operation)?;
    let _guard = state.operation.lock().await;
    tauri::async_runtime::spawn_blocking(move || jlink::target_status(&request))
        .await
        .map_err(|error| McuError::operation(format!("J-Link 状态任务失败：{error}")))?
}

#[command]
pub async fn mcu_jlink_target_action(
    board_id: String,
    request: jlink::JLinkActionRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let action = request.action;
    let board = board_by_id(&app, &board_id)?;
    validate_jlink_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 目标控制").map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "目标控制", "J-Link")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        serde_json::json!({"backend": "jLink"}),
    )?;
    if matches!(action, TargetAction::ResetRun | TargetAction::ResetHalt) {
        release_board_uart(&app, &board)?;
    }
    let _guard = state.operation.lock().await;
    let result = tauri::async_runtime::spawn_blocking(move || jlink::target_action(&request))
        .await
        .map_err(|error| McuError::operation(format!("J-Link 控制任务失败：{error}")))
        .and_then(|result| result);
    publish_mcu_result(
        &app,
        &board.id,
        McuAuditActor::User,
        "target_control",
        target_action_name(action),
        &audit_intent_id,
        &result,
    );
    result
}

#[command]
pub async fn mcu_jlink_flash_internal(
    board_id: String,
    request: jlink::JLinkFlashRequest,
    app: tauri::AppHandle,
    state: State<'_, McuState>,
) -> Result<OperationResult, McuError> {
    let board = board_by_id(&app, &board_id)?;
    validate_jlink_binding(&board, &request.connection)?;
    crate::hardware_safety::ensure_idle(&app, "MCU 内部 Flash 烧写")
        .map_err(McuError::operation)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("MCU 工作台", "内部 Flash 烧写", "J-Link")
        .map_err(McuError::operation)?;
    let audit_intent_id = publish_mcu_intent(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "internal",
        serde_json::json!({
            "backend": "jLink", "address": request.bin_start_address,
            "postAction": flash_post_action_name(request.post_action),
        }),
    )?;
    release_board_uart(&app, &board)?;
    let _guard = state.operation.lock().await;
    let result = tauri::async_runtime::spawn_blocking(move || jlink::flash_internal(&request))
        .await
        .map_err(|error| McuError::operation(format!("J-Link 内部 Flash 烧写任务失败：{error}")))
        .and_then(|result| result);
    publish_mcu_result(
        &app,
        &board.id,
        McuAuditActor::User,
        "flash",
        "internal",
        &audit_intent_id,
        &result,
    );
    result
}

fn power_action_name(action: McuPowerAction) -> &'static str {
    match action {
        McuPowerAction::On => "on",
        McuPowerAction::Off => "off",
        McuPowerAction::Cycle => "cycle",
    }
}

pub(crate) fn release_board_uart(
    app: &tauri::AppHandle,
    board: &McuBoardConfig,
) -> Result<(), McuError> {
    let identity = crate::serial::commands::SerialIdentity {
        port: board.serial_port.clone(),
        vid: board.serial_vid,
        pid: board.serial_pid,
        serial_number: board.serial_number.clone(),
        interface_number: board.serial_interface_number,
    };
    let port = crate::serial::commands::resolve_port_core(&identity)
        .map(|port| port.name)
        .unwrap_or_else(|_| board.serial_port.clone());
    if port.trim().is_empty() {
        return Ok(());
    }
    let state = app.state::<Arc<crate::serial::SerialState>>();
    if crate::serial::commands::serial_is_registered_core(state.inner().as_ref(), &port) {
        crate::serial::commands::serial_close_core(state.inner().as_ref(), &port).map_err(
            |error| McuError::operation(format!("MCU_UART_RELEASE_FAILED:{port}:{error}")),
        )?;
    }
    Ok(())
}

pub(crate) fn target_action_name(action: TargetAction) -> &'static str {
    match action {
        TargetAction::ResetRun => "reset_run",
        TargetAction::ResetHalt => "reset_halt",
        TargetAction::Run => "run",
        TargetAction::Halt => "halt",
    }
}

fn flash_post_action_name(action: FlashPostAction) -> &'static str {
    match action {
        FlashPostAction::None => "none",
        FlashPostAction::ResetRun => "reset_run",
        FlashPostAction::ResetHalt => "reset_halt",
    }
}

fn validate_power_binding(
    board: &McuBoardConfig,
    request: &McuPowerRequest,
) -> Result<(), McuError> {
    if board.power_controller_type != "ch347_gpio"
        || board.power_device_index != request.device_index
        || board.power_gpio != request.gpio
        || board.power_active_high != request.active_high
    {
        return Err(McuError::invalid("MCU_POWER_BINDING_MISMATCH"));
    }
    Ok(())
}

fn option_matches_saved(saved: &str, provided: Option<&str>) -> bool {
    saved.trim() == provided.unwrap_or_default().trim()
}

fn validate_stlink_binding(
    board: &McuBoardConfig,
    connection: &StLinkConnection,
) -> Result<(), McuError> {
    if !matches!(board.probe_backend, ProbeBackend::CubeProgrammer)
        || !option_matches_saved(&board.tool_path, connection.executable_path.as_deref())
        || !option_matches_saved(&board.probe_serial, connection.probe_serial.as_deref())
        || board.swd_frequency_khz != connection.frequency_khz
    {
        return Err(McuError::invalid("MCU_PROBE_BINDING_MISMATCH"));
    }
    Ok(())
}

fn validate_openocd_binding(
    board: &McuBoardConfig,
    connection: &openocd::OpenOcdConnection,
) -> Result<(), McuError> {
    if !matches!(board.probe_backend, ProbeBackend::OpenOcd)
        || !option_matches_saved(&board.tool_path, connection.executable_path.as_deref())
        || !option_matches_saved(&board.probe_serial, connection.probe_serial.as_deref())
        || board.swd_frequency_khz != connection.frequency_khz
        || board.openocd_interface_config != connection.interface_config
        || board.openocd_target_config != connection.target_config
        || board.openocd_board_config != connection.board_config
    {
        return Err(McuError::invalid("MCU_PROBE_BINDING_MISMATCH"));
    }
    Ok(())
}

fn validate_pyocd_binding(
    board: &McuBoardConfig,
    connection: &pyocd::PyOcdConnection,
) -> Result<(), McuError> {
    if !matches!(board.probe_backend, ProbeBackend::PyOcd)
        || !option_matches_saved(&board.tool_path, connection.executable_path.as_deref())
        || !option_matches_saved(&board.probe_serial, connection.probe_serial.as_deref())
        || board.swd_frequency_khz != connection.frequency_khz
        || board.pyocd_target != connection.target
        || board.pyocd_pack_path != connection.pack_path
    {
        return Err(McuError::invalid("MCU_PROBE_BINDING_MISMATCH"));
    }
    Ok(())
}

fn validate_jlink_binding(
    board: &McuBoardConfig,
    connection: &jlink::JLinkConnection,
) -> Result<(), McuError> {
    if !matches!(board.probe_backend, ProbeBackend::JLink)
        || !option_matches_saved(&board.tool_path, connection.executable_path.as_deref())
        || !option_matches_saved(&board.probe_serial, connection.probe_serial.as_deref())
        || board.swd_frequency_khz != connection.frequency_khz
        || board.jlink_device != connection.device
    {
        return Err(McuError::invalid("MCU_PROBE_BINDING_MISMATCH"));
    }
    Ok(())
}

pub(crate) fn publish_mcu_intent(
    app: &tauri::AppHandle,
    board_id: &str,
    actor: McuAuditActor,
    category: &str,
    operation: &str,
    metadata: serde_json::Value,
) -> Result<String, McuError> {
    let (actor_type, actor_id) = actor.identity();
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: format!("mcu_{category}"),
            action: format!("mcu_{category}_intended"),
            scope_type: "mcu_board".to_owned(),
            scope_id: format!("mcu:{board_id}"),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: None,
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: format!("准备执行 MCU 受控动作：{operation}"),
            metadata: serde_json::json!({"operation": operation, "parameters": metadata}),
        },
    )
    .map_err(McuError::operation)?;
    match outcome {
        crate::audit::AuditWriteOutcome::Recorded { event_id } => Ok(event_id),
        crate::audit::AuditWriteOutcome::Degraded { .. } => {
            Err(McuError::operation("AUDIT_REQUIRED_DEGRADED"))
        }
    }
}

pub(crate) fn publish_mcu_result<T, E: std::fmt::Display>(
    app: &tauri::AppHandle,
    board_id: &str,
    actor: McuAuditActor,
    category: &str,
    operation: &str,
    causation_id: &str,
    result: &Result<T, E>,
) {
    let (actor_type, actor_id) = actor.identity();
    let (status, error_code) = match result {
        Ok(_) => ("success", None),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                &error.to_string(),
                "MCU_OPERATION_FAILED",
            )),
        ),
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: format!("mcu_{category}"),
            action: format!("mcu_{category}_completed"),
            scope_type: "mcu_board".to_owned(),
            scope_id: format!("mcu:{board_id}"),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: None,
            causation_id: Some(causation_id.to_owned()),
            result: status.to_owned(),
            error_code,
            summary: format!(
                "MCU 受控动作{}：{operation}",
                if result.is_ok() { "完成" } else { "失败" }
            ),
            metadata: serde_json::json!({"operation": operation}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

#[cfg(test)]
mod audit_tests {
    use super::*;

    #[test]
    fn target_action_names_are_stable_and_probe_binding_is_board_scoped() {
        assert_eq!(target_action_name(TargetAction::ResetHalt), "reset_halt");
        let board = McuBoardConfig {
            id: "board-1".to_owned(),
            probe_backend: ProbeBackend::PyOcd,
            pyocd_target: "r7fa6m5bh".to_owned(),
            swd_frequency_khz: 1_000,
            ..McuBoardConfig::default()
        };
        let connection = pyocd::PyOcdConnection {
            executable_path: None,
            probe_serial: None,
            frequency_khz: 1_000,
            target: "r7fa6m5bh".to_owned(),
            pack_path: String::new(),
        };
        assert!(validate_pyocd_binding(&board, &connection).is_ok());
        let mut wrong = connection;
        wrong.target = "stm32h750".to_owned();
        assert_eq!(
            validate_pyocd_binding(&board, &wrong).unwrap_err().message,
            "MCU_PROBE_BINDING_MISMATCH"
        );

        let jlink_board = McuBoardConfig {
            id: "board-jlink".to_owned(),
            probe_backend: ProbeBackend::JLink,
            probe_serial: "123456789".to_owned(),
            swd_frequency_khz: 4_000,
            jlink_device: "STM32H750ZBT6".to_owned(),
            ..McuBoardConfig::default()
        };
        let jlink_connection = jlink::JLinkConnection {
            executable_path: None,
            probe_serial: Some("123456789".to_owned()),
            frequency_khz: 4_000,
            device: "STM32H750ZBT6".to_owned(),
        };
        assert!(validate_jlink_binding(&jlink_board, &jlink_connection).is_ok());
        let mut wrong_jlink = jlink_connection;
        wrong_jlink.device = "STM32H743ZI".to_owned();
        assert_eq!(
            validate_jlink_binding(&jlink_board, &wrong_jlink)
                .unwrap_err()
                .message,
            "MCU_PROBE_BINDING_MISMATCH"
        );
    }
}

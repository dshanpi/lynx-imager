use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Manager, State};

use super::commands::McuError;
use super::McuState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McuBoardConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub board_image_path: String,
    #[serde(default)]
    pub chip_vendor: String,
    #[serde(default)]
    pub chip_series: String,
    pub mcu_model: String,
    pub probe_backend: ProbeBackend,
    pub probe_serial: String,
    pub serial_port: String,
    #[serde(default)]
    pub serial_vid: u16,
    #[serde(default)]
    pub serial_pid: u16,
    #[serde(default)]
    pub serial_number: String,
    #[serde(default)]
    pub serial_interface_number: Option<u8>,
    pub baud_rate: u32,
    pub tool_path: String,
    pub swd_frequency_khz: u32,
    pub openocd_interface_config: String,
    pub openocd_target_config: String,
    pub openocd_board_config: String,
    pub openocd_flash_bank: String,
    #[serde(default)]
    pub pyocd_target: String,
    #[serde(default)]
    pub pyocd_pack_path: String,
    #[serde(default)]
    pub jlink_device: String,
    pub external_loader_path: String,
    pub ospi_address_start: u64,
    pub ospi_size_bytes: u64,
    #[serde(default)]
    pub external_flash_model: String,
    #[serde(default)]
    pub external_sram_model: String,
    #[serde(default)]
    pub uart_instance: String,
    #[serde(default)]
    pub uart_tx_pin: String,
    #[serde(default)]
    pub uart_rx_pin: String,
    #[serde(default)]
    pub boot_success_text: String,
    #[serde(default)]
    pub boot_failure_text: String,
    #[serde(default = "default_boot_timeout_ms")]
    pub boot_timeout_ms: u64,
    #[serde(default)]
    pub mcp_host_address: String,
    #[serde(default = "default_ssh_port")]
    pub mcp_host_port: u16,
    #[serde(default = "default_ssh_username")]
    pub mcp_host_username: String,
    #[serde(default = "default_ssh_auth_type")]
    pub mcp_auth_type: String,
    #[serde(default)]
    pub mcp_private_key_path: String,
    #[serde(default)]
    pub mcp_host_fingerprint: String,
    #[serde(default = "default_mcp_local_port")]
    pub mcp_local_port: u16,
    #[serde(default = "default_mcp_remote_port")]
    pub mcp_remote_port: u16,
    #[serde(default)]
    pub mcp_server_name: String,
    #[serde(default)]
    pub mcu_sdk_dir: String,
    #[serde(default)]
    pub mcu_build_command: String,
    #[serde(default)]
    pub mcu_firmware_path: String,
    #[serde(default)]
    pub power_controller_type: String,
    #[serde(default)]
    pub power_device_index: u32,
    #[serde(default = "default_power_gpio")]
    pub power_gpio: u8,
    #[serde(default = "default_true")]
    pub power_active_high: bool,
}

fn default_boot_timeout_ms() -> u64 {
    30_000
}

fn default_power_gpio() -> u8 {
    4
}

fn default_mcp_local_port() -> u16 {
    8767
}
fn default_mcp_remote_port() -> u16 {
    28767
}
fn default_ssh_port() -> u16 {
    22
}
fn default_ssh_username() -> String {
    "ubuntu".to_owned()
}
fn default_ssh_auth_type() -> String {
    "password".to_owned()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeBackend {
    OpenOcd,
    PyOcd,
    CubeProgrammer,
    JLink,
}

impl Default for McuBoardConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            board_image_path: String::new(),
            chip_vendor: "st".to_owned(),
            chip_series: "stm32h7".to_owned(),
            mcu_model: "STM32H750ZBT6".to_owned(),
            probe_backend: ProbeBackend::OpenOcd,
            probe_serial: String::new(),
            serial_port: String::new(),
            serial_vid: 0,
            serial_pid: 0,
            serial_number: String::new(),
            serial_interface_number: None,
            baud_rate: 115_200,
            tool_path: String::new(),
            swd_frequency_khz: 1_000,
            openocd_interface_config: "interface/stlink.cfg".to_owned(),
            openocd_target_config: "target/stm32h7x.cfg".to_owned(),
            openocd_board_config: String::new(),
            openocd_flash_bank: "stm32h7x.bank1.cpu0".to_owned(),
            pyocd_target: String::new(),
            pyocd_pack_path: String::new(),
            jlink_device: String::new(),
            external_loader_path: String::new(),
            ospi_address_start: 0,
            ospi_size_bytes: 0,
            external_flash_model: String::new(),
            external_sram_model: String::new(),
            uart_instance: String::new(),
            uart_tx_pin: String::new(),
            uart_rx_pin: String::new(),
            boot_success_text: String::new(),
            boot_failure_text: String::new(),
            boot_timeout_ms: default_boot_timeout_ms(),
            mcp_host_address: String::new(),
            mcp_host_port: default_ssh_port(),
            mcp_host_username: "ubuntu".to_owned(),
            mcp_auth_type: "password".to_owned(),
            mcp_private_key_path: String::new(),
            mcp_host_fingerprint: String::new(),
            mcp_local_port: default_mcp_local_port(),
            mcp_remote_port: default_mcp_remote_port(),
            mcp_server_name: "lynx-mcu-board".to_owned(),
            mcu_sdk_dir: String::new(),
            mcu_build_command: String::new(),
            mcu_firmware_path: String::new(),
            power_controller_type: String::new(),
            power_device_index: 0,
            power_gpio: default_power_gpio(),
            power_active_high: true,
        }
    }
}

#[command]
pub async fn mcu_list_boards(
    app: AppHandle,
    state: State<'_, McuState>,
) -> Result<Vec<McuBoardConfig>, McuError> {
    let _guard = state.configuration.lock().await;
    list_boards(&app)
}

#[command]
pub async fn mcu_save_board(
    mut board: McuBoardConfig,
    app: AppHandle,
    state: State<'_, McuState>,
) -> Result<McuBoardConfig, McuError> {
    normalize_chip_identity(&mut board);
    validate_board(&board)?;
    let _guard = state.configuration.lock().await;
    if board.id.trim().is_empty() {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| McuError::operation(format!("系统时间无效：{error}")))?
            .as_millis();
        board.id = format!("mcu-{millis}-{:08x}", rand::random::<u32>());
    }
    validate_id(&board.id)?;
    ensure_unique_mcp_local_port(&board, &list_boards(&app)?)?;
    let old = list_boards(&app)?
        .into_iter()
        .find(|existing| existing.id == board.id);
    let fields = crate::config_audit::changed_fields(
        old.as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            .as_ref(),
        serde_json::to_value(&board).ok().as_ref(),
        MCU_AUDIT_FIELDS,
    );
    let operation = if old.is_some() { "update" } else { "create" };
    let audit = crate::config_audit::begin(&app, "mcu_board", format!("mcu:{}", board.id), "mcu_board", operation, fields, serde_json::json!({"probeBackend": board.probe_backend, "mcpConfigured": !board.mcp_host_address.is_empty(), "powerBound": !board.power_controller_type.is_empty()})).map_err(McuError::operation)?;
    let directory = boards_dir(&app)?;
    fs::create_dir_all(&directory)
        .map_err(|error| McuError::operation(format!("创建 MCU 配置目录失败：{error}")))?;
    let path = directory.join(format!("{}.json", board.id));
    let encoded = serde_json::to_vec_pretty(&board)
        .map_err(|error| McuError::operation(format!("序列化 MCU 配置失败：{error}")))?;
    let result = write_config(&path, &encoded).map(|()| board);
    let audit_result = result
        .as_ref()
        .map(|_| ())
        .map_err(|error| error.to_string());
    crate::config_audit::finish(&app, &audit, &audit_result);
    result
}

#[command]
pub async fn mcu_delete_board(
    id: String,
    app: AppHandle,
    state: State<'_, McuState>,
) -> Result<(), McuError> {
    validate_id(&id)?;
    let _guard = state.configuration.lock().await;
    let audit = crate::config_audit::begin(
        &app,
        "mcu_board",
        format!("mcu:{id}"),
        "mcu_board",
        "delete",
        vec![],
        serde_json::json!({}),
    )
    .map_err(McuError::operation)?;
    let path = boards_dir(&app)?.join(format!("{id}.json"));
    let result = if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| McuError::operation(format!("删除 MCU 配置失败：{error}")))
    } else {
        Ok(())
    };
    let audit_result = result
        .as_ref()
        .map(|_| ())
        .map_err(|error| error.to_string());
    crate::config_audit::finish(&app, &audit, &audit_result);
    result
}

const MCU_AUDIT_FIELDS: &[&str] = &[
    "name",
    "boardImagePath",
    "chipVendor",
    "chipSeries",
    "mcuModel",
    "probeBackend",
    "probeSerial",
    "serialPort",
    "baudRate",
    "swdFrequencyKhz",
    "mcpHostAddress",
    "mcpHostPort",
    "mcpHostUsername",
    "mcpAuthType",
    "mcpPrivateKeyPath",
    "mcpHostFingerprint",
    "mcpLocalPort",
    "mcpRemotePort",
    "mcpServerName",
    "mcuSdkDir",
    "mcuBuildCommand",
    "mcuFirmwarePath",
    "powerControllerType",
    "powerDeviceIndex",
    "powerGpio",
    "powerActiveHigh",
];

pub(crate) fn list_boards(app: &AppHandle) -> Result<Vec<McuBoardConfig>, McuError> {
    let directory = boards_dir(app)?;
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut boards = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| McuError::operation(format!("读取 MCU 配置目录失败：{error}")))?
    {
        let entry =
            entry.map_err(|error| McuError::operation(format!("读取 MCU 配置项失败：{error}")))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let data = fs::read(&path).map_err(|error| {
            McuError::operation(format!("读取 {} 失败：{error}", path.display()))
        })?;
        let mut board: McuBoardConfig = serde_json::from_slice(&data).map_err(|error| {
            McuError::operation(format!("解析 {} 失败：{error}", path.display()))
        })?;
        normalize_chip_identity(&mut board);
        if board.name.chars().count() > 20 {
            board.name = board.name.chars().take(20).collect();
        }
        if !(4..=7).contains(&board.power_gpio) {
            board.power_gpio = default_power_gpio();
        }
        validate_board(&board)?;
        boards.push(board);
    }
    boards.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(boards)
}

/// Resolve one MCU board by its stable id. MCU-domain services use this on
/// every request so a long-lived MCP session never retains stale pin/tool
/// configuration.
pub(crate) fn board_by_id(app: &AppHandle, id: &str) -> Result<McuBoardConfig, McuError> {
    validate_id(id)?;
    list_boards(app)?
        .into_iter()
        .find(|board| board.id == id)
        .ok_or_else(|| McuError::invalid(format!("MCU 板卡不存在：{id}")))
}

fn boards_dir(app: &AppHandle) -> Result<PathBuf, McuError> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("mcu-boards"))
        .map_err(|error| McuError::operation(format!("无法获取应用数据目录：{error}")))
}

fn write_config(path: &Path, data: &[u8]) -> Result<(), McuError> {
    let temporary = path.with_extension(format!("json.tmp-{:08x}", rand::random::<u32>()));
    let backup = path.with_extension(format!("json.bak-{:08x}", rand::random::<u32>()));
    fs::write(&temporary, data)
        .map_err(|error| McuError::operation(format!("写入 MCU 临时配置失败：{error}")))?;
    let had_previous = path.exists();
    if had_previous {
        fs::rename(path, &backup).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            McuError::operation(format!("备份原 MCU 配置失败：{error}"))
        })?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        if had_previous {
            let _ = fs::rename(&backup, path);
        }
        return Err(McuError::operation(format!("保存 MCU 配置失败：{error}")));
    }
    if had_previous {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

fn validate_board(board: &McuBoardConfig) -> Result<(), McuError> {
    if !board.id.is_empty() {
        validate_id(&board.id)?;
    }
    if board.name.trim().is_empty() || board.name.chars().count() > 20 {
        return Err(McuError::invalid(
            "MCU 板卡名称不能为空且不能超过 20 个字符",
        ));
    }
    if board.mcu_model.trim().is_empty() || board.mcu_model.chars().count() > 80 {
        return Err(McuError::invalid("MCU 型号不能为空且不能超过 80 个字符"));
    }
    let supported_identity = matches!(
        (board.chip_vendor.as_str(), board.chip_series.as_str()),
        ("st", "stm32f1" | "stm32f4" | "stm32h7") | ("renesas", "ra6" | "ra8")
    );
    if !supported_identity {
        return Err(McuError::invalid(
            "芯片厂家与系列不在 LYNX 已验证支持列表中",
        ));
    }
    let model = board.mcu_model.trim().to_ascii_uppercase();
    let model_matches_series = match board.chip_series.as_str() {
        "stm32f1" => model.starts_with("STM32F1"),
        "stm32f4" => model.starts_with("STM32F4"),
        "stm32h7" => model.starts_with("STM32H7"),
        "ra6" => model.starts_with("RA6") || model.starts_with("R7FA6"),
        "ra8" => model.starts_with("RA8") || model.starts_with("R7FA8"),
        _ => false,
    };
    if !model_matches_series {
        return Err(McuError::invalid("具体 MCU 料号与所选芯片系列不匹配"));
    }
    if !(5..=50_000).contains(&board.swd_frequency_khz) {
        return Err(McuError::invalid("SWD 频率必须在 5 到 50000 kHz 之间"));
    }
    if board.baud_rate == 0 || board.baud_rate > 12_000_000 {
        return Err(McuError::invalid("串口波特率超出有效范围"));
    }
    if !(1_000..=600_000).contains(&board.boot_timeout_ms) {
        return Err(McuError::invalid("启动检测超时必须在 1 到 600 秒之间"));
    }
    if board.mcp_local_port == 0 || board.mcp_remote_port == 0 {
        return Err(McuError::invalid("MCU MCP 端口不能为 0"));
    }
    if board.mcp_host_port == 0 {
        return Err(McuError::invalid("MCU MCP SSH 端口不能为 0"));
    }
    if !(4..=7).contains(&board.power_gpio) {
        return Err(McuError::invalid("Lynx Power GPIO 必须在 4 到 7 之间"));
    }
    if !board.mcp_server_name.is_empty()
        && !board
            .mcp_server_name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-'))
    {
        return Err(McuError::invalid(
            "MCU MCP 名称只能包含 ASCII 字母、数字、下划线和连字符",
        ));
    }
    for (label, value) in [
        ("MCU SDK 目录", &board.mcu_sdk_dir),
        ("MCU 编译命令", &board.mcu_build_command),
        ("MCU 固件路径", &board.mcu_firmware_path),
    ] {
        if value.contains('\0') {
            return Err(McuError::invalid(format!("{label}不能包含 NUL 字符")));
        }
    }
    if board.boot_success_text.chars().count() > 256
        || board.boot_failure_text.chars().count() > 256
    {
        return Err(McuError::invalid("启动判定文本不能超过 256 个字符"));
    }
    for (label, value) in [
        ("外部 Flash 型号", &board.external_flash_model),
        ("外部 SRAM 型号", &board.external_sram_model),
        ("UART 实例", &board.uart_instance),
        ("UART TX 引脚", &board.uart_tx_pin),
        ("UART RX 引脚", &board.uart_rx_pin),
        ("pyOCD 目标型号", &board.pyocd_target),
        ("J-Link Device", &board.jlink_device),
    ] {
        if value.chars().count() > 80 {
            return Err(McuError::invalid(format!("{label}不能超过 80 个字符")));
        }
    }
    if matches!(board.probe_backend, ProbeBackend::PyOcd) && board.pyocd_target.trim().is_empty() {
        return Err(McuError::invalid("pyOCD 目标型号不能为空"));
    }
    if matches!(board.probe_backend, ProbeBackend::JLink)
        && (board.jlink_device.trim().is_empty()
            || !board
                .jlink_device
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.')))
    {
        return Err(McuError::invalid(
            "J-Link Device 不能为空且只能包含安全字符",
        ));
    }
    if !board.openocd_flash_bank.is_empty()
        && !board
            .openocd_flash_bank
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
    {
        return Err(McuError::invalid("OpenOCD Flash Bank 名称包含非法字符"));
    }
    Ok(())
}

fn ensure_unique_mcp_local_port(
    board: &McuBoardConfig,
    existing: &[McuBoardConfig],
) -> Result<(), McuError> {
    if let Some(conflict) = existing
        .iter()
        .find(|item| item.id != board.id && item.mcp_local_port == board.mcp_local_port)
    {
        return Err(McuError::invalid(format!(
            "MCU MCP 本地端口 {} 已由板卡“{}”使用；每块 MCU 单板必须使用独立端口",
            board.mcp_local_port, conflict.name
        )));
    }
    Ok(())
}

fn normalize_chip_identity(board: &mut McuBoardConfig) {
    if !board.chip_vendor.trim().is_empty() && !board.chip_series.trim().is_empty() {
        return;
    }
    let model = board.mcu_model.trim().to_ascii_uppercase();
    let (vendor, series) = if model.starts_with("STM32F1") {
        ("st", "stm32f1")
    } else if model.starts_with("STM32F4") {
        ("st", "stm32f4")
    } else if model.starts_with("STM32H7") {
        ("st", "stm32h7")
    } else if model.starts_with("RA8") || model.starts_with("R7FA8") {
        ("renesas", "ra8")
    } else if model.starts_with("RA6") || model.starts_with("R7FA6") {
        ("renesas", "ra6")
    } else {
        ("st", "stm32h7")
    };
    board.chip_vendor = vendor.to_owned();
    board.chip_series = series.to_owned();
}

fn validate_id(id: &str) -> Result<(), McuError> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
    {
        return Err(McuError::invalid("MCU 配置 ID 格式无效"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_board_template_has_no_saved_device_or_example_peripherals() {
        let board = McuBoardConfig::default();
        assert!(matches!(board.probe_backend, ProbeBackend::OpenOcd));
        assert_eq!(board.openocd_interface_config, "interface/stlink.cfg");
        assert_eq!(board.power_gpio, 4);
        assert!(board.id.is_empty());
        assert!(board.name.is_empty());
        assert!(board.serial_port.is_empty());
        assert!(board.probe_serial.is_empty());
        assert!(board.external_flash_model.is_empty());
        assert!(board.external_sram_model.is_empty());
        assert!(board.uart_instance.is_empty());
    }

    #[test]
    fn rejects_path_traversal_id() {
        assert!(validate_id("../linux-board").is_err());
    }

    #[test]
    fn rejects_duplicate_mcu_mcp_local_port() {
        let existing = McuBoardConfig {
            id: "board-a".to_owned(),
            name: "Board A".to_owned(),
            mcp_local_port: 8767,
            ..Default::default()
        };
        let candidate = McuBoardConfig {
            id: "board-b".to_owned(),
            name: "Board B".to_owned(),
            mcp_local_port: 8767,
            ..Default::default()
        };
        let error = ensure_unique_mcp_local_port(&candidate, &[existing])
            .expect_err("duplicate local port must be rejected");
        assert!(error.to_string().contains("8767"));
    }

    #[test]
    fn old_board_json_receives_boot_observation_defaults() {
        let json = r#"{
          "id":"mcu-old","name":"Legacy","mcuModel":"STM32H750XBH6",
          "probeBackend":"openOcd","probeSerial":"","serialPort":"",
          "baudRate":115200,"toolPath":"","swdFrequencyKhz":4000,
          "openocdInterfaceConfig":"interface/stlink.cfg",
          "openocdTargetConfig":"target/stm32h7x.cfg","openocdBoardConfig":"",
          "openocdFlashBank":"ospi","externalLoaderPath":"",
          "ospiAddressStart":2415919104,"ospiSizeBytes":67108864
        }"#;
        let board: McuBoardConfig = serde_json::from_str(json).unwrap();
        assert!(board.boot_success_text.is_empty());
        assert!(board.boot_failure_text.is_empty());
        assert_eq!(board.boot_timeout_ms, 30_000);
        assert!(board.external_flash_model.is_empty());
        assert!(board.uart_instance.is_empty());
        assert_eq!(board.power_gpio, 4);
        assert!(board.chip_vendor.is_empty());
        assert_eq!(board.serial_vid, 0);
        assert_eq!(board.serial_pid, 0);
        assert!(board.serial_number.is_empty());
        assert_eq!(board.serial_interface_number, None);
        assert!(board.board_image_path.is_empty());
        assert!(board.jlink_device.is_empty());

        let mut board = board;
        normalize_chip_identity(&mut board);
        assert_eq!(board.chip_vendor, "st");
        assert_eq!(board.chip_series, "stm32h7");
    }

    #[test]
    fn infers_legacy_ra6_board_identity() {
        let mut board = McuBoardConfig::default();
        board.chip_vendor.clear();
        board.chip_series.clear();
        board.mcu_model = "R7FA6M5BF2CBG".to_owned();
        normalize_chip_identity(&mut board);
        assert_eq!(board.chip_vendor, "renesas");
        assert_eq!(board.chip_series, "ra6");
    }

    #[test]
    fn rejects_model_outside_selected_series() {
        let board = McuBoardConfig {
            chip_series: "stm32f1".to_owned(),
            ..Default::default()
        };
        assert!(validate_board(&board).is_err());
    }
}

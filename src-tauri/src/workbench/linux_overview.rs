use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{command, AppHandle, Manager, State};

use crate::adb::commands::AdbState;
use crate::serial::commands::{
    resolve_port_core, serial_is_open_core, serial_read_core, serial_write_core, SerialIdentity,
};
use crate::serial::SerialState;

use super::types::DeviceConfig;
use super::{credentials, storage, WorkbenchState};

/// Fixed, read-only diagnostics used by the Linux workbench overview. The
/// selected board contributes only its persisted ADB serial; no device value is
/// interpolated into this command.
pub const LINUX_OVERVIEW_COMMAND: &str = r#"
printf 'hostname='; (hostname 2>/dev/null || cat /proc/sys/kernel/hostname 2>/dev/null) | head -n 1
printf 'os_name='; sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | head -n 1 | tr -d '"'
printf 'kernel='; uname -r 2>/dev/null
printf 'architecture='; uname -m 2>/dev/null
printf 'cpu_model='; awk -F ': *' '/^(model name|Hardware|Processor)[[:space:]]*:/ { print $2; exit }' /proc/cpuinfo 2>/dev/null
awk '/^processor[[:space:]]*:/ { cores++ } END { if (cores > 0) print "cpu_cores=" cores }' /proc/cpuinfo 2>/dev/null
awk '/^cpu / { total=0; for (i=2; i<=NF; i++) total += $i; print "cpu_total_ticks=" total; print "cpu_idle_ticks=" ($5 + $6); exit }' /proc/stat 2>/dev/null
awk '{ print "load_one=" $1; print "load_five=" $2; print "load_fifteen=" $3; exit }' /proc/loadavg 2>/dev/null
awk '/^MemTotal:/ { print "memory_total_kib=" $2 } /^MemAvailable:/ { print "memory_available_kib=" $2 }' /proc/meminfo 2>/dev/null
awk '{ split($1, value, "."); print "uptime_seconds=" value[1]; exit }' /proc/uptime 2>/dev/null
df -kP / 2>/dev/null | awk 'NR == 2 { print "storage_total_kib=" $2; print "storage_available_kib=" $4; exit }'
for zone in /sys/class/thermal/thermal_zone*/temp; do if [ -r "$zone" ]; then printf 'temperature_millicelsius='; head -n 1 "$zone"; break; fi; done
printf 'ip_addresses='; ip -o -4 addr show scope global 2>/dev/null | awk '{ value=$4; sub(/\/.*/, "", value); printf "%s%s", separator, value; separator="," } END { print "" }'
"#;

const ADB_OVERVIEW_TIMEOUT: Duration = Duration::from_secs(8);
const ADB_OVERVIEW_OUTPUT_LIMIT: usize = 256 * 1024;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxDeviceOverview {
    pub device_id: i64,
    pub adb_serial: String,
    pub transport: String,
    pub transport_detail: String,
    pub collected_at_ms: u64,
    pub hostname: String,
    pub os_name: String,
    pub kernel: String,
    pub architecture: String,
    pub cpu_model: String,
    pub cpu_cores: Option<u32>,
    pub cpu_usage_pct: Option<f64>,
    pub load_one: Option<f64>,
    pub load_five: Option<f64>,
    pub load_fifteen: Option<f64>,
    pub memory_total_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub storage_total_bytes: Option<u64>,
    pub storage_available_bytes: Option<u64>,
    pub temperature_celsius: Option<f64>,
    pub uptime_seconds: Option<u64>,
    pub ip_addresses: Vec<String>,
    #[serde(skip)]
    cpu_total_ticks: Option<u64>,
    #[serde(skip)]
    cpu_idle_ticks: Option<u64>,
}

fn text_value(value: &str) -> String {
    value.trim().chars().take(512).collect()
}

fn kib_to_bytes(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()?.checked_mul(1024)
}

fn parse_overview(output: &str) -> LinuxDeviceOverview {
    let mut overview = LinuxDeviceOverview::default();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Interactive embedded shells may print a prompt before each command
        // result even when terminal echo is disabled (for example
        // `# memory_total_kib=735236`). Only the final whitespace-delimited
        // token is considered and it must still match the fixed allow-list.
        let normalized_key = key.split_whitespace().last().unwrap_or_default();
        match normalized_key {
            "hostname" => overview.hostname = text_value(value),
            "os_name" => overview.os_name = text_value(value),
            "kernel" => overview.kernel = text_value(value),
            "architecture" => overview.architecture = text_value(value),
            "cpu_model" => overview.cpu_model = text_value(value),
            "cpu_cores" => overview.cpu_cores = value.trim().parse().ok(),
            "cpu_total_ticks" => overview.cpu_total_ticks = value.trim().parse().ok(),
            "cpu_idle_ticks" => overview.cpu_idle_ticks = value.trim().parse().ok(),
            "load_one" => overview.load_one = value.trim().parse().ok(),
            "load_five" => overview.load_five = value.trim().parse().ok(),
            "load_fifteen" => overview.load_fifteen = value.trim().parse().ok(),
            "memory_total_kib" => overview.memory_total_bytes = kib_to_bytes(value),
            "memory_available_kib" => overview.memory_available_bytes = kib_to_bytes(value),
            "storage_total_kib" => overview.storage_total_bytes = kib_to_bytes(value),
            "storage_available_kib" => overview.storage_available_bytes = kib_to_bytes(value),
            "temperature_millicelsius" => {
                overview.temperature_celsius = value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|temperature| temperature / 1000.0)
                    .filter(|temperature| (-50.0..=250.0).contains(temperature));
            }
            "uptime_seconds" => overview.uptime_seconds = value.trim().parse().ok(),
            "ip_addresses" => {
                overview.ip_addresses = value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .take(8)
                    .map(text_value)
                    .collect();
            }
            _ => {}
        }
    }
    overview
}

fn cpu_usage_between(first: &LinuxDeviceOverview, second: &LinuxDeviceOverview) -> Option<f64> {
    let total_delta = second
        .cpu_total_ticks?
        .checked_sub(first.cpu_total_ticks?)?;
    let idle_delta = second.cpu_idle_ticks?.checked_sub(first.cpu_idle_ticks?)?;
    if total_delta == 0 || idle_delta > total_delta {
        return None;
    }
    Some((((total_delta - idle_delta) as f64 / total_delta as f64) * 100.0).clamp(0.0, 100.0))
}

fn framed_output<'a>(output: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let end_at = output.rfind(end)?;
    let begin_at = output[..end_at].rfind(begin)? + begin.len();
    Some(&output[begin_at..end_at])
}

fn overview_has_data(overview: &LinuxDeviceOverview) -> bool {
    !overview.hostname.is_empty()
        || !overview.kernel.is_empty()
        || !overview.architecture.is_empty()
        || !overview.cpu_model.is_empty()
        || overview.cpu_cores.is_some()
        || overview.cpu_total_ticks.is_some()
        || overview.memory_total_bytes.is_some()
        || overview.storage_total_bytes.is_some()
        || overview.temperature_celsius.is_some()
        || overview.uptime_seconds.is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SerialConsoleState {
    LinuxShell,
    LoginRequired,
    PasswordRequired,
    RtSmart,
    Unknown,
}

fn serial_console_state(output: &str, device: &DeviceConfig) -> SerialConsoleState {
    let normalized = output.replace('\r', "");
    let lower = normalized.to_ascii_lowercase();

    // K230 boards ship with RT-Smart by default. Its msh console accepts text
    // but is not a POSIX shell, so a Linux telemetry script can wedge it.
    if device.serial_console_type == "rt_smart"
        || (device.serial_console_type == "auto" && device.chip_vendor == "kendryte")
        || lower.contains("rt-smart")
        || lower.contains("rt-thread")
        || lower.contains("msh />")
        || lower.contains("[i/ntp]")
    {
        return SerialConsoleState::RtSmart;
    }

    let last_line = normalized
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let last_lower = last_line.to_ascii_lowercase();
    if last_line.ends_with(device.serial_login_prompt.trim())
        || last_lower.ends_with("login:")
        || last_lower.ends_with("username:")
        || last_lower.ends_with("user:")
    {
        return SerialConsoleState::LoginRequired;
    }
    if last_line.ends_with(device.serial_password_prompt.trim())
        || last_lower.ends_with("password:")
    {
        return SerialConsoleState::PasswordRequired;
    }
    if last_line.ends_with(device.serial_shell_prompt.trim())
        || last_line.ends_with('#')
        || last_line.ends_with('$')
    {
        return SerialConsoleState::LinuxShell;
    }
    SerialConsoleState::Unknown
}

fn passive_console_error(state: SerialConsoleState) -> Result<(), String> {
    match state {
        SerialConsoleState::LinuxShell => Ok(()),
        SerialConsoleState::LoginRequired => Err(
            "SERIAL_OVERVIEW_LOGIN_REQUIRED:目标串口正在等待用户名；请在串口页登录，或使用已配置凭据的受控串口登录流程，成功进入 Shell 后再刷新"
                .to_owned(),
        ),
        SerialConsoleState::PasswordRequired => Err(
            "SERIAL_OVERVIEW_PASSWORD_REQUIRED:目标串口正在等待密码；设备概览不会自动发送或猜测凭据，请先完成受控登录"
                .to_owned(),
        ),
        SerialConsoleState::RtSmart => Err(
            "SERIAL_OVERVIEW_RT_SMART_UNSUPPORTED:检测到 Kendryte/RT-Smart 控制台；为避免串口卡死，设备概览不会发送 Linux POSIX 探测命令"
                .to_owned(),
        ),
        SerialConsoleState::Unknown => Err(
            "SERIAL_OVERVIEW_SHELL_NOT_CONFIRMED:未从现有串口输出确认 Linux Shell 提示符；LYNX 未发送任何探测命令，请先完成登录并停留在 Shell 提示符后刷新"
                .to_owned(),
        ),
    }
}

fn wait_for_console_state(
    state: &SerialState,
    port: &str,
    device: &DeviceConfig,
    deadline: std::time::Instant,
) -> Result<SerialConsoleState, String> {
    let mut output = Vec::new();
    while std::time::Instant::now() < deadline {
        output.extend(serial_read_core(state, port, 64 * 1024)?);
        if output.len() > 64 * 1024 {
            output.drain(..output.len() - 64 * 1024);
        }
        let console_state = serial_console_state(&String::from_utf8_lossy(&output), device);
        if console_state != SerialConsoleState::Unknown {
            return Ok(console_state);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err("SERIAL_LOGIN_TIMEOUT:等待目标串口登录提示符或 Shell 超时".to_owned())
}

fn prepare_linux_serial_shell(
    state: &SerialState,
    port: &str,
    device: &DeviceConfig,
    passive_output: &[u8],
) -> Result<(), String> {
    let mut console_state = serial_console_state(&String::from_utf8_lossy(passive_output), device);
    if console_state == SerialConsoleState::LinuxShell {
        return Ok(());
    }
    if !device.serial_auto_login {
        return passive_console_error(console_state);
    }
    if device.serial_console_type == "rt_smart" || device.serial_login_mode == "none" {
        return passive_console_error(console_state);
    }

    let deadline = std::time::Instant::now()
        + Duration::from_millis(device.serial_reconnect_timeout_ms.clamp(1_000, 30_000));
    if console_state == SerialConsoleState::LoginRequired {
        serial_write_core(
            state,
            port,
            format!("{}\n", device.serial_login_username).as_bytes(),
        )?;
        console_state = wait_for_console_state(state, port, device, deadline)?;
    }
    if console_state == SerialConsoleState::PasswordRequired {
        if device.serial_login_mode != "password" {
            return Err(
                "SERIAL_LOGIN_PASSWORD_NOT_CONFIGURED:目标要求密码，但当前单板仅配置了用户名登录"
                    .to_owned(),
            );
        }
        let device_id = device
            .id
            .ok_or_else(|| "SERIAL_LOGIN_DEVICE_INVALID".to_owned())?;
        let password = credentials::resolve_serial_login_password(device_id)?.ok_or_else(|| {
            "SERIAL_LOGIN_CREDENTIAL_MISSING:请在单板设置中保存串口登录密码".to_owned()
        })?;
        serial_write_core(state, port, format!("{password}\n").as_bytes())?;
        console_state = wait_for_console_state(state, port, device, deadline)?;
    }
    if console_state == SerialConsoleState::LinuxShell {
        Ok(())
    } else {
        passive_console_error(console_state)
    }
}

fn serial_snapshot(
    state: &SerialState,
    port: &str,
    device: &DeviceConfig,
    nonce: u128,
) -> Result<LinuxDeviceOverview, String> {
    let begin = format!("__LYNX_OVERVIEW_BEGIN_{nonce}__");
    let end = format!("__LYNX_OVERVIEW_END_{nonce}__");
    let passive_output = serial_read_core(state, port, 64 * 1024)?;
    prepare_linux_serial_shell(state, port, device, &passive_output)?;
    // Do not mutate terminal flags with `stty`. Echoed command text remains
    // outside the unique frame and the parser accepts only fixed keys.
    let script = format!(
        "printf '\\n{begin}\\n'\n{}\nprintf '{end}\\n'\n",
        LINUX_OVERVIEW_COMMAND
    );
    serial_write_core(state, port, script.as_bytes())?;

    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut output = Vec::new();
    while std::time::Instant::now() < deadline {
        output.extend(serial_read_core(state, port, 64 * 1024)?);
        let text = String::from_utf8_lossy(&output);
        if let Some(payload) = framed_output(&text, &begin, &end) {
            let overview = parse_overview(payload);
            if overview_has_data(&overview) {
                return Ok(overview);
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err("SERIAL_OVERVIEW_SHELL_UNAVAILABLE:串口已连接，但目标机尚未进入可执行 Shell；请先登录后刷新".to_owned())
}

#[command]
pub async fn workbench_linux_device_overview(
    device_id: i64,
    app: AppHandle,
    workbench: State<'_, Arc<WorkbenchState>>,
    adb: State<'_, AdbState>,
    serial: State<'_, Arc<SerialState>>,
) -> Result<LinuxDeviceOverview, String> {
    let device = {
        let db = workbench.db.lock().map_err(|error| error.to_string())?;
        storage::list_devices(&db)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|device| device.id == Some(device_id))
            .ok_or_else(|| format!("BOARD_NOT_FOUND:未找到 Linux 单板 {device_id}"))?
    };
    saved_device_overview(
        device,
        app,
        &adb,
        serial.inner(),
        OverviewAccess::Configured,
    )
    .await
}

#[derive(Clone, Copy)]
pub(crate) enum OverviewAccess {
    Configured,
    PassiveBound,
}

fn overview_port(
    resolved: Result<String, String>,
    configured: String,
    open: bool,
    access: OverviewAccess,
) -> Result<String, String> {
    match resolved {
        Ok(port) => Ok(port),
        Err(error) if matches!(access, OverviewAccess::PassiveBound) => Err(error),
        Err(_) if open => Ok(configured),
        Err(_) => Err("SERIAL_OVERVIEW_NOT_OPEN:请先打开该单板的串口并登录目标 Shell".into()),
    }
}

/// Backend-only adapter for a persisted, verified device snapshot. Callers must
/// resolve the snapshot from their own saved scope; no public IPC accepts it.
pub(crate) async fn saved_device_overview(
    device: DeviceConfig,
    app: AppHandle,
    adb: &AdbState,
    serial: &Arc<SerialState>,
    access: OverviewAccess,
) -> Result<LinuxDeviceOverview, String> {
    let device_id = device.id.ok_or("BOARD_NOT_FOUND:板卡尚未保存")?;
    let adb_serial = device.adb_serial.trim().to_owned();

    crate::hardware_safety::ensure_idle(&app, "读取 Linux 单板状态")?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin(
        "Linux Workbench",
        "读取目标机状态",
        format!("linux:{device_id}"),
    )?;
    let adb_result = if adb_serial.is_empty() {
        None
    } else {
        let manager = adb.0.lock().map_err(|error| error.to_string())?.clone();
        let serial_for_read = adb_serial.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                let devices = manager.list_devices()?;
                let bound = devices
                    .iter()
                    .find(|candidate| candidate.serial == serial_for_read)
                    .ok_or_else(|| "ADB_DEVICE_OFFLINE:已绑定的 ADB 设备当前不在线".to_owned())?;
                if bound.state != "device" {
                    return Err(format!(
                        "ADB_DEVICE_UNAVAILABLE:已绑定的 ADB 设备状态为 {}",
                        bound.state
                    ));
                }
                let first = parse_overview(&manager.shell_command_with_timeout(
                    Some(&serial_for_read),
                    LINUX_OVERVIEW_COMMAND,
                    ADB_OVERVIEW_TIMEOUT,
                    ADB_OVERVIEW_OUTPUT_LIMIT,
                )?);
                std::thread::sleep(Duration::from_millis(250));
                let second = parse_overview(&manager.shell_command_with_timeout(
                    Some(&serial_for_read),
                    LINUX_OVERVIEW_COMMAND,
                    ADB_OVERVIEW_TIMEOUT,
                    ADB_OVERVIEW_OUTPUT_LIMIT,
                )?);
                Ok::<_, String>((first, second, "adb".to_owned(), serial_for_read))
            })
            .await
            .map_err(|error| format!("ADB_OVERVIEW_JOIN_FAILED:{error}"))?,
        )
    };

    let (first, mut overview, transport, transport_detail) = match adb_result {
        Some(Ok(value)) => value,
        _ => {
            let identity = SerialIdentity {
                port: device.serial_port.clone(),
                vid: device.serial_vid,
                pid: device.serial_pid,
                serial_number: device.serial_number.clone(),
                interface_number: device.serial_interface_number,
            };
            let configured_port = device.serial_port.clone();
            let port = overview_port(
                resolve_port_core(&identity).map(|resolved| resolved.name),
                configured_port.clone(),
                serial_is_open_core(serial, &configured_port),
                access,
            )?;
            if !serial_is_open_core(serial, &port) {
                return Err(
                    "SERIAL_OVERVIEW_NOT_OPEN:请先打开该单板的串口并登录目标 Shell".to_owned(),
                );
            }
            let serial_state = serial.clone();
            let mut serial_device = device.clone();
            serial_device.serial_auto_login &= matches!(access, OverviewAccess::Configured);
            tokio::task::spawn_blocking(move || {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let first = serial_snapshot(&serial_state, &port, &serial_device, nonce)?;
                std::thread::sleep(Duration::from_millis(250));
                let second = serial_snapshot(
                    &serial_state,
                    &port,
                    &serial_device,
                    nonce.saturating_add(1),
                )?;
                Ok::<_, String>((first, second, "serial".to_owned(), port))
            })
            .await
            .map_err(|error| format!("SERIAL_OVERVIEW_JOIN_FAILED:{error}"))??
        }
    };

    overview.device_id = device_id;
    overview.adb_serial = adb_serial;
    overview.transport = transport;
    overview.transport_detail = transport_detail;
    overview.collected_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    overview.cpu_usage_pct = cpu_usage_between(&first, &overview);
    Ok(overview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passive_bound_observation_never_uses_an_unresolved_open_port() {
        let unresolved = || Err("SERIAL_IDENTITY_AMBIGUOUS".into());
        assert!(overview_port(
            unresolved(),
            "unverified-open-port".into(),
            true,
            OverviewAccess::PassiveBound
        )
        .is_err());
        assert_eq!(
            overview_port(
                Ok("verified-port".into()),
                "other-open-port".into(),
                true,
                OverviewAccess::PassiveBound
            )
            .unwrap(),
            "verified-port"
        );
        assert_eq!(
            overview_port(
                unresolved(),
                "configured-port".into(),
                true,
                OverviewAccess::Configured
            )
            .unwrap(),
            "configured-port"
        );
        assert!(overview_port(
            unresolved(),
            "closed-port".into(),
            false,
            OverviewAccess::Configured
        )
        .is_err());
    }

    fn console_device(vendor: &str) -> DeviceConfig {
        DeviceConfig {
            id: Some(1),
            board_uid: "linux-test".to_owned(),
            name: "test".to_owned(),
            board_image_path: String::new(),
            chip_vendor: vendor.to_owned(),
            serial_port: "test".to_owned(),
            serial_vid: 0,
            serial_pid: 0,
            serial_number: String::new(),
            usb_binding: String::new(),
            camera_binding: String::new(),
            adb_serial: String::new(),
            serial_interface_number: None,
            baud_rate: 115_200,
            serial_console_type: "auto".to_owned(),
            serial_login_mode: "none".to_owned(),
            serial_login_username: "root".to_owned(),
            serial_login_prompt: "login:".to_owned(),
            serial_password_prompt: "Password:".to_owned(),
            serial_shell_prompt: "#".to_owned(),
            serial_auto_login: false,
            flasher_kind: String::new(),
            flasher_path: String::new(),
            flasher_args: String::new(),
            power_controller: String::new(),
            power_controller_vid: 0,
            power_controller_pid: 0,
            power_controller_serial: String::new(),
            power_controller_interface_number: None,
            power_baud_rate: 115_200,
            power_profile_json: String::new(),
            power_channel: String::new(),
            boot_channel: String::new(),
            reset_channel: String::new(),
            power_off_wait_ms: 1_000,
            power_on_wait_ms: 2_000,
            serial_reconnect_timeout_ms: 30_000,
            boot_timeout_ms: 60_000,
        }
    }

    #[test]
    fn parses_real_linux_overview_fields_and_units() {
        let parsed = parse_overview(
            "hostname=t153-dev\n\
             os_name=Ubuntu 24.04.1 LTS\n\
             kernel=6.6.44\n\
             architecture=aarch64\n\
             cpu_model=ARMv8 Processor rev 4\n\
             cpu_cores=4\n\
             cpu_total_ticks=1000\n\
             cpu_idle_ticks=750\n\
             load_one=0.42\n\
             load_five=0.35\n\
             load_fifteen=0.20\n\
             memory_total_kib=1048576\n\
             memory_available_kib=524288\n\
             storage_total_kib=8388608\n\
             storage_available_kib=4194304\n\
             temperature_millicelsius=48750\n\
             uptime_seconds=86401\n\
             ip_addresses=192.168.1.20,10.0.0.3\n",
        );
        assert_eq!(parsed.hostname, "t153-dev");
        assert_eq!(parsed.cpu_cores, Some(4));
        assert_eq!(parsed.memory_total_bytes, Some(1_073_741_824));
        assert_eq!(parsed.storage_available_bytes, Some(4_294_967_296));
        assert_eq!(parsed.temperature_celsius, Some(48.75));
        assert_eq!(parsed.ip_addresses, ["192.168.1.20", "10.0.0.3"]);
    }

    #[test]
    fn calculates_cpu_usage_from_two_target_snapshots() {
        let first = parse_overview("cpu_total_ticks=1000\ncpu_idle_ticks=800\n");
        let second = parse_overview("cpu_total_ticks=1200\ncpu_idle_ticks=900\n");
        assert_eq!(cpu_usage_between(&first, &second), Some(50.0));
    }

    #[test]
    fn rejects_invalid_temperature_and_counter_regression() {
        let first = parse_overview(
            "cpu_total_ticks=2000\ncpu_idle_ticks=1200\ntemperature_millicelsius=999999\n",
        );
        let second = parse_overview("cpu_total_ticks=1000\ncpu_idle_ticks=900\n");
        assert_eq!(first.temperature_celsius, None);
        assert_eq!(cpu_usage_between(&first, &second), None);
    }

    #[test]
    fn extracts_last_complete_serial_frame_without_shell_echo() {
        let output = "root# printf '__LYNX_OVERVIEW_BEGIN_7__'\n__LYNX_OVERVIEW_BEGIN_7__\nhostname=t153\nroot# printf '__LYNX_OVERVIEW_END_7__'\n__LYNX_OVERVIEW_END_7__\n";
        let payload = framed_output(
            output,
            "__LYNX_OVERVIEW_BEGIN_7__",
            "__LYNX_OVERVIEW_END_7__",
        )
        .unwrap();
        assert_eq!(parse_overview(payload).hostname, "t153");
    }

    #[test]
    fn echo_only_serial_frame_is_not_accepted_as_telemetry() {
        let parsed =
            parse_overview("root# printf 'hostname='; hostname\nroot# printf 'cpu_total_ticks='\n");
        assert!(!overview_has_data(&parsed));
    }

    #[test]
    fn parses_serial_values_prefixed_by_interactive_shell_prompt() {
        let parsed = parse_overview(
            "# hostname=t153\n# kernel=5.10.198\n# cpu_total_ticks=988436\n# memory_total_kib=735236\n# temperature_millicelsius=41712\n",
        );
        assert_eq!(parsed.hostname, "t153");
        assert_eq!(parsed.kernel, "5.10.198");
        assert_eq!(parsed.cpu_total_ticks, Some(988436));
        assert_eq!(parsed.memory_total_bytes, Some(752_881_664));
        assert_eq!(parsed.temperature_celsius, Some(41.712));
    }

    #[test]
    fn blocks_k230_rt_smart_before_any_linux_serial_command() {
        let mut device = console_device("kendryte");
        assert_eq!(
            serial_console_state("[I/ntp] Get local time from NTP server\r\nmsh /> ", &device),
            SerialConsoleState::RtSmart
        );
        assert!(
            passive_console_error(serial_console_state("msh /> ", &device))
                .unwrap_err()
                .starts_with("SERIAL_OVERVIEW_RT_SMART_UNSUPPORTED:")
        );
        device.serial_console_type = "linux".to_owned();
        assert_eq!(
            serial_console_state("root@k230-linux:~# ", &device),
            SerialConsoleState::LinuxShell
        );
    }

    #[test]
    fn requires_login_completion_before_linux_serial_probe() {
        let allwinner = console_device("allwinner");
        let rockchip = console_device("rockchip");
        let nxp = console_device("nxp");
        assert_eq!(
            serial_console_state("Welcome\r\ndevice login: ", &allwinner),
            SerialConsoleState::LoginRequired
        );
        assert_eq!(
            serial_console_state("Password: ", &rockchip),
            SerialConsoleState::PasswordRequired
        );
        assert_eq!(
            serial_console_state("root@board:~# ", &nxp),
            SerialConsoleState::LinuxShell
        );
        assert_eq!(
            serial_console_state("booting kernel...", &allwinner),
            SerialConsoleState::Unknown
        );
    }
}

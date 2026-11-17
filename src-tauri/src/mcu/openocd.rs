use std::env;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::process::configure_background_command;

use super::commands::{FlashPostAction, McuError, OperationResult, TargetAction, TargetStatus};
use super::cube_programmer::{validate_external_range, validate_firmware_addresses};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const FLASH_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOcdToolRequest {
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOcdToolInfo {
    pub available: bool,
    pub executable_path: String,
    pub version: String,
    pub bundled: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOcdConnection {
    pub executable_path: Option<String>,
    pub probe_serial: Option<String>,
    pub frequency_khz: u32,
    pub interface_config: String,
    pub target_config: String,
    pub board_config: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOcdActionRequest {
    pub connection: OpenOcdConnection,
    pub action: TargetAction,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOcdFlashRequest {
    pub connection: OpenOcdConnection,
    pub firmware_path: String,
    pub flash_bank: String,
    pub external_address_start: u64,
    pub external_address_size: u64,
    pub bin_start_address: Option<u64>,
    pub post_action: FlashPostAction,
}

pub fn tool_info(
    app: &AppHandle,
    request: &OpenOcdToolRequest,
) -> Result<OpenOcdToolInfo, McuError> {
    let resolved = resolve_executable(app, request.executable_path.as_deref())?;
    let output = run_command(&resolved.path, &["--version".to_owned()], PROBE_TIMEOUT)?;
    ensure_success(&output, "检测 OpenOCD")?;
    Ok(OpenOcdToolInfo {
        available: true,
        executable_path: resolved.path.to_string_lossy().into_owned(),
        version: combined_output(&output).trim().to_owned(),
        bundled: resolved.bundled,
    })
}

pub fn target_status(
    app: &AppHandle,
    connection: &OpenOcdConnection,
) -> Result<TargetStatus, McuError> {
    let resolved = resolve_executable(app, connection.executable_path.as_deref())?;
    let mut args = session_args(connection)?;
    command(&mut args, "init");
    command(&mut args, "targets");
    command(&mut args, "mdw 0x5c001000 1");
    command(&mut args, "shutdown");
    let output = run_command(&resolved.path, &args, ACTION_TIMEOUT)?;
    ensure_success(&output, "读取 OpenOCD 目标状态")?;
    let text = combined_output(&output);
    Ok(parse_target_status(&text))
}

pub fn target_action(
    app: &AppHandle,
    request: &OpenOcdActionRequest,
) -> Result<OperationResult, McuError> {
    let resolved = resolve_executable(app, request.connection.executable_path.as_deref())?;
    let mut args = session_args(&request.connection)?;
    command(&mut args, "init");
    for action_command in target_action_commands(request.action) {
        command(&mut args, action_command);
    }
    command(&mut args, "shutdown");
    let output = run_command(&resolved.path, &args, ACTION_TIMEOUT)?;
    operation_result(output, "OpenOCD 目标控制")
}

fn target_action_commands(action: TargetAction) -> &'static [&'static str] {
    match action {
        TargetAction::ResetRun => &["reset run"],
        TargetAction::ResetHalt => &["reset halt", "wait_halt 1000"],
        // OpenOCD is intentionally short-lived. A fresh process must first
        // capture the halted core context before it can restore and resume it.
        TargetAction::Run => &["halt", "wait_halt 1000", "resume"],
        TargetAction::Halt => &["halt", "wait_halt 1000"],
    }
}

pub fn flash_ospi(
    app: &AppHandle,
    request: &OpenOcdFlashRequest,
) -> Result<OperationResult, McuError> {
    if request.connection.board_config.trim().is_empty() {
        return Err(McuError::invalid(
            "OpenOCD 烧写外部 OSPI 必须提供板级配置文件；该文件需要初始化 OCTOSPI 并声明 stmqspi flash bank",
        ));
    }
    validate_flash_bank(&request.flash_bank)?;
    validate_external_range(
        request.external_address_start,
        request.external_address_size,
    )?;
    let firmware = canonical_file(&request.firmware_path, "固件")?;
    validate_firmware_addresses(
        &firmware,
        request.external_address_start,
        request.external_address_size,
        request.bin_start_address,
    )?;

    let resolved = resolve_executable(app, request.connection.executable_path.as_deref())?;
    let mut args = session_args(&request.connection)?;
    command(&mut args, "init");
    command(&mut args, "reset init");
    command(&mut args, &format!("flash probe {}", request.flash_bank));
    let firmware_path = tcl_path(&firmware)?;
    let mut program = format!("program {firmware_path} verify");
    if extension_eq(&firmware, "bin") {
        let address = request
            .bin_start_address
            .ok_or_else(|| McuError::invalid("BIN 固件必须设置 OSPI 起始地址"))?;
        program.push_str(&format!(" 0x{address:08X}"));
    }
    command(&mut args, &program);
    command(
        &mut args,
        match request.post_action {
            FlashPostAction::None => "halt",
            FlashPostAction::ResetRun => "reset run",
            FlashPostAction::ResetHalt => "reset halt",
        },
    );
    command(&mut args, "shutdown");
    let output = run_command(&resolved.path, &args, FLASH_TIMEOUT)?;
    operation_result(output, "OpenOCD OSPI 烧写与校验")
}

struct ResolvedExecutable {
    path: PathBuf,
    bundled: bool,
}

fn resolve_executable(
    app: &AppHandle,
    configured: Option<&str>,
) -> Result<ResolvedExecutable, McuError> {
    if let Some(value) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        let path = canonical_file(value, "OpenOCD")?;
        validate_executable_name(&path)?;
        return Ok(ResolvedExecutable {
            path,
            bundled: false,
        });
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        for name in ["openocd", "openocd.exe"] {
            for candidate in [
                resource_dir.join("openocd").join("bin").join(name),
                resource_dir.join("bin").join(name),
            ] {
                if candidate.is_file() {
                    return Ok(ResolvedExecutable {
                        path: candidate.canonicalize().map_err(|error| {
                            McuError::operation(format!("解析内置 OpenOCD 路径失败：{error}"))
                        })?,
                        bundled: true,
                    });
                }
            }
        }
    }

    for name in ["openocd", "openocd.exe"] {
        if let Some(path) = find_in_path(name) {
            return Ok(ResolvedExecutable {
                path,
                bundled: false,
            });
        }
    }
    Err(McuError::tool_not_found(
        "未找到 OpenOCD。请使用 LYNX 内置 OpenOCD 运行包，或手动选择 openocd 可执行文件。",
    ))
}

fn validate_executable_name(path: &Path) -> Result<(), McuError> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(name.as_str(), "openocd" | "openocd.exe") {
        Ok(())
    } else {
        Err(McuError::invalid(
            "OpenOCD 可执行文件名必须是 openocd 或 openocd.exe",
        ))
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
            .and_then(|candidate| candidate.canonicalize().ok())
    })
}

fn session_args(connection: &OpenOcdConnection) -> Result<Vec<String>, McuError> {
    if !(5..=50_000).contains(&connection.frequency_khz) {
        return Err(McuError::invalid("SWD 频率必须在 5 到 50000 kHz 之间"));
    }
    let mut args = vec![
        "-f".to_owned(),
        validate_config(&connection.interface_config, false)?,
    ];
    if connection.board_config.trim().is_empty() {
        args.extend([
            "-f".to_owned(),
            validate_config(&connection.target_config, false)?,
        ]);
    } else {
        args.extend([
            "-f".to_owned(),
            validate_config(&connection.board_config, true)?,
        ]);
    }
    command(&mut args, "gdb_port disabled");
    command(&mut args, "tcl_port disabled");
    command(&mut args, "telnet_port disabled");
    command(
        &mut args,
        &format!("adapter speed {}", connection.frequency_khz),
    );
    if let Some(serial) = connection
        .probe_serial
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if serial.len() > 96
            || !serial
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
        {
            return Err(McuError::invalid("探针序列号包含非法字符"));
        }
        command(&mut args, &format!("adapter serial {serial}"));
    }
    Ok(args)
}

fn validate_config(value: &str, require_file: bool) -> Result<String, McuError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(McuError::invalid("OpenOCD 配置文件不能为空"));
    }
    let path = Path::new(trimmed);
    if path.is_file() {
        return path
            .canonicalize()
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|error| McuError::invalid(format!("解析 OpenOCD 配置路径失败：{error}")));
    }
    if require_file {
        return Err(McuError::invalid(format!(
            "OpenOCD 板级配置文件不存在：{trimmed}"
        )));
    }
    if trimmed.contains("..")
        || trimmed.starts_with(['/', '\\'])
        || !trimmed
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '/' | '_' | '-' | '.'))
    {
        return Err(McuError::invalid("OpenOCD 内置配置名称格式无效"));
    }
    Ok(trimmed.to_owned())
}

fn validate_flash_bank(value: &str) -> Result<(), McuError> {
    if value.is_empty()
        || value.len() > 80
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        Err(McuError::invalid("OpenOCD Flash Bank 名称格式无效"))
    } else {
        Ok(())
    }
}

fn canonical_file(value: &str, label: &str) -> Result<PathBuf, McuError> {
    let path = Path::new(value.trim());
    if !path.is_file() {
        return Err(McuError::invalid(format!(
            "{label}文件不存在：{}",
            path.display()
        )));
    }
    path.canonicalize()
        .map_err(|error| McuError::invalid(format!("无法解析{label}路径：{error}")))
}

fn extension_eq(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn tcl_path(path: &Path) -> Result<String, McuError> {
    let value = path.to_string_lossy();
    if value.contains(['{', '}', '\n', '\r']) {
        return Err(McuError::invalid("文件路径包含 OpenOCD Tcl 不支持的字符"));
    }
    Ok(format!("{{{value}}}"))
}

fn command(args: &mut Vec<String>, value: &str) {
    args.extend(["-c".to_owned(), value.to_owned()]);
}

fn run_command(executable: &Path, args: &[String], timeout: Duration) -> Result<Output, McuError> {
    let mut process = Command::new(executable);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_background_command(&mut process);
    let mut child = process
        .spawn()
        .map_err(|error| McuError::operation(format!("无法启动 OpenOCD：{error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McuError::operation("无法读取 OpenOCD 标准输出"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| McuError::operation("无法读取 OpenOCD 错误输出"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|error| McuError::operation(format!("等待 OpenOCD 失败：{error}")))?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(McuError::timeout(format!(
                    "OpenOCD 操作超时（{} 秒）",
                    timeout.as_secs()
                )));
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| McuError::operation("OpenOCD 输出读取线程异常"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| McuError::operation("OpenOCD 错误读取线程异常"))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_all(mut reader: impl Read) -> Result<Vec<u8>, McuError> {
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .map_err(|error| McuError::operation(format!("读取 OpenOCD 输出失败：{error}")))?;
    Ok(output)
}

fn combined_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout.into_owned(),
        (true, false) => stderr.into_owned(),
        (true, true) => String::new(),
    }
}

fn ensure_success(output: &Output, operation: &str) -> Result<(), McuError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(McuError::operation(format!(
            "{operation}失败（退出码 {}）：{}",
            output.status.code().unwrap_or(-1),
            combined_output(output).trim()
        )))
    }
}

fn operation_result(output: Output, operation: &str) -> Result<OperationResult, McuError> {
    ensure_success(&output, operation)?;
    Ok(OperationResult {
        success: true,
        exit_code: output.status.code().unwrap_or(0),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn parse_target_status(text: &str) -> TargetStatus {
    let lower = text.to_ascii_lowercase();
    let core_state = if lower.contains("halted") {
        "HALTED"
    } else if lower.contains("running") {
        "RUNNING"
    } else if lower.contains("reset") {
        "RESET"
    } else {
        "CONNECTED"
    };
    let voltage = text.lines().find_map(|line| {
        let (_, value) = line.split_once("Target voltage:")?;
        value.split_whitespace().next()?.parse::<f64>().ok()
    });
    let device_id = text
        .lines()
        .find_map(|line| {
            let (_, value) = line.split_once("0x5c001000:")?;
            let raw = u32::from_str_radix(value.split_whitespace().next()?, 16).ok()?;
            Some(format!("0x{:03X} · rev 0x{:04X}", raw & 0xFFF, raw >> 16))
        })
        .unwrap_or_default();
    TargetStatus {
        connected: true,
        device_name: "STM32H7".to_owned(),
        device_id,
        voltage,
        core_state: core_state.to_owned(),
        output: text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> OpenOcdConnection {
        OpenOcdConnection {
            executable_path: None,
            probe_serial: Some("55FF1234".to_owned()),
            frequency_khz: 4_000,
            interface_config: "interface/stlink.cfg".to_owned(),
            target_config: "target/stm32h7x.cfg".to_owned(),
            board_config: String::new(),
        }
    }

    #[test]
    fn builds_openocd_arguments_without_shell() {
        let args = session_args(&connection()).unwrap();
        assert_eq!(
            args,
            [
                "-f",
                "interface/stlink.cfg",
                "-f",
                "target/stm32h7x.cfg",
                "-c",
                "gdb_port disabled",
                "-c",
                "tcl_port disabled",
                "-c",
                "telnet_port disabled",
                "-c",
                "adapter speed 4000",
                "-c",
                "adapter serial 55FF1234",
            ]
        );
    }

    #[test]
    fn rejects_tcl_path_injection() {
        assert!(tcl_path(Path::new("firmware}; shutdown; {.bin")).is_err());
    }

    #[test]
    fn requires_real_board_config_for_ospi() {
        assert!(validate_config("board/missing.cfg", true).is_err());
    }

    #[test]
    fn run_recaptures_context_in_the_same_openocd_session() {
        assert_eq!(
            target_action_commands(TargetAction::Run),
            ["halt", "wait_halt 1000", "resume"]
        );
        assert_eq!(
            target_action_commands(TargetAction::Halt),
            ["halt", "wait_halt 1000"]
        );
    }
}

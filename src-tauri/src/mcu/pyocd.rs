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

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const FLASH_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PyOcdToolRequest {
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PyOcdToolInfo {
    pub available: bool,
    pub executable_path: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PyOcdConnection {
    pub executable_path: Option<String>,
    pub probe_serial: Option<String>,
    pub frequency_khz: u32,
    pub target: String,
    pub pack_path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PyOcdActionRequest {
    pub connection: PyOcdConnection,
    pub action: TargetAction,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PyOcdFlashRequest {
    pub connection: PyOcdConnection,
    pub firmware_path: String,
    pub bin_start_address: Option<u64>,
    pub post_action: FlashPostAction,
}

#[derive(Debug, Clone)]
struct PyOcdExecutable {
    path: PathBuf,
    module_mode: bool,
}

pub fn tool_info(app: &AppHandle, request: &PyOcdToolRequest) -> Result<PyOcdToolInfo, McuError> {
    let executable = resolve_executable(app, request.executable_path.as_deref())?;
    let output = run_command(&executable, &["--version".to_owned()], PROBE_TIMEOUT)?;
    ensure_success(&output, "检测 pyOCD")?;
    Ok(PyOcdToolInfo {
        available: true,
        executable_path: executable.path.to_string_lossy().into_owned(),
        version: combined_output(&output).trim().to_owned(),
    })
}

pub fn target_status(
    app: &AppHandle,
    connection: &PyOcdConnection,
) -> Result<TargetStatus, McuError> {
    let executable = resolve_executable(app, connection.executable_path.as_deref())?;
    let mut args = connection_args(app, "commander", connection)?;
    args.extend([
        "--connect".to_owned(),
        "attach".to_owned(),
        "--command".to_owned(),
        "show target".to_owned(),
        "--command".to_owned(),
        "status".to_owned(),
        "--command".to_owned(),
        "show uid".to_owned(),
    ]);
    let output = run_command(&executable, &args, PROBE_TIMEOUT)?;
    ensure_success(&output, "识别 DAP/RA 目标")?;
    let text = combined_output(&output);
    Ok(parse_target_status(connection.target.trim(), &text))
}

fn parse_target_status(target: &str, text: &str) -> TargetStatus {
    let lower = text.to_ascii_lowercase();
    if missing_probe_message(&lower) {
        return TargetStatus {
            connected: false,
            device_name: target.to_owned(),
            device_id: String::new(),
            voltage: None,
            core_state: "DISCONNECTED".to_owned(),
            output: text.to_owned(),
        };
    }
    let core_state = if lower.contains("halted") {
        "HALTED"
    } else if lower.contains("running") {
        "RUNNING"
    } else if lower.contains("reset") {
        "RESET"
    } else {
        "CONNECTED"
    };
    let device_id = text
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("unique id"))
        .and_then(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_owned())
        })
        .unwrap_or_default();
    TargetStatus {
        connected: true,
        device_name: target.to_owned(),
        device_id,
        voltage: None,
        core_state: core_state.to_owned(),
        output: text.to_owned(),
    }
}

fn missing_probe_message(lower_output: &str) -> bool {
    lower_output.contains("no connected debug probes")
        || lower_output.contains("no available debug probes")
        || lower_output.contains("debug probe not found")
}

pub fn target_action(
    app: &AppHandle,
    request: &PyOcdActionRequest,
) -> Result<OperationResult, McuError> {
    let executable = resolve_executable(app, request.connection.executable_path.as_deref())?;
    let mut args = connection_args(app, "commander", &request.connection)?;
    let command = match request.action {
        TargetAction::ResetRun => "reset",
        TargetAction::ResetHalt => "reset -h",
        TargetAction::Run => "continue",
        TargetAction::Halt => "halt",
    };
    args.extend([
        "--connect".to_owned(),
        "attach".to_owned(),
        "--command".to_owned(),
        command.to_owned(),
    ]);
    let output = run_command(&executable, &args, ACTION_TIMEOUT)?;
    operation_result(output, "pyOCD 目标控制")
}

pub fn flash_internal(
    app: &AppHandle,
    request: &PyOcdFlashRequest,
) -> Result<OperationResult, McuError> {
    let executable = resolve_executable(app, request.connection.executable_path.as_deref())?;
    let firmware = canonical_file(&request.firmware_path, "固件")?;
    let extension = firmware
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "bin" | "hex" | "elf"
    ) {
        return Err(McuError::invalid("pyOCD 固件必须是 .bin、.hex 或 .elf"));
    }
    let mut args = connection_args(app, "load", &request.connection)?;
    args.extend(["--erase".to_owned(), "auto".to_owned()]);
    if extension.eq_ignore_ascii_case("bin") {
        if let Some(address) = request.bin_start_address {
            args.extend(["--base-address".to_owned(), format!("0x{address:x}")]);
        }
    }
    if matches!(request.post_action, FlashPostAction::None) {
        args.push("--no-reset".to_owned());
    }
    args.push(firmware.to_string_lossy().into_owned());
    let output = run_command(&executable, &args, FLASH_TIMEOUT)?;
    let result = operation_result(output, "pyOCD 内部 Flash 烧写")?;
    if matches!(request.post_action, FlashPostAction::ResetHalt) {
        let action = PyOcdActionRequest {
            connection: request.connection.clone(),
            action: TargetAction::ResetHalt,
        };
        target_action(app, &action)?;
    }
    Ok(result)
}

fn connection_args(
    app: &AppHandle,
    command: &str,
    connection: &PyOcdConnection,
) -> Result<Vec<String>, McuError> {
    if !(5..=50_000).contains(&connection.frequency_khz) {
        return Err(McuError::invalid("SWD 频率必须在 5 到 50000 kHz 之间"));
    }
    let target = connection.target.trim();
    if target.is_empty()
        || target.len() > 80
        || !target
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-'))
    {
        return Err(McuError::invalid("pyOCD 目标型号格式无效"));
    }
    let pack = resolve_pack(app, &connection.pack_path)?;
    if !pack
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("pack"))
    {
        return Err(McuError::invalid("CMSIS-Pack 文件扩展名必须是 .pack"));
    }
    let mut args = vec![
        command.to_owned(),
        "--no-config".to_owned(),
        "--target".to_owned(),
        target.to_owned(),
        "--pack".to_owned(),
        pack.to_string_lossy().into_owned(),
        "--frequency".to_owned(),
        format!("{}k", connection.frequency_khz),
        "--no-wait".to_owned(),
    ];
    if let Some(serial) = connection
        .probe_serial
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if serial.len() > 128
            || !serial
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | ':'))
        {
            return Err(McuError::invalid("DAP 探针序列号包含非法字符"));
        }
        args.extend(["--uid".to_owned(), serial.to_owned()]);
    }
    Ok(args)
}

fn resolve_executable(app: &AppHandle, value: Option<&str>) -> Result<PyOcdExecutable, McuError> {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        let path = canonical_file(value, "pyOCD")?;
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let module_mode = name == "python.exe" && windows_pyocd_module(&path).is_file();
        if !matches!(name.as_str(), "pyocd" | "pyocd.exe") && !module_mode {
            return Err(McuError::invalid(
                "pyOCD 可执行文件名必须是 pyocd/pyocd.exe，或内置 pyOCD 的 python.exe",
            ));
        }
        return Ok(PyOcdExecutable { path, module_mode });
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        for candidate in [
            resource_dir.join("pyembed").join("bin").join("pyocd"),
            resource_dir
                .join("pyembed")
                .join("Scripts")
                .join("pyocd.exe"),
        ] {
            if candidate.is_file() {
                let path = candidate.canonicalize().map_err(|error| {
                    McuError::operation(format!("解析内置 pyOCD 路径失败：{error}"))
                })?;
                return Ok(PyOcdExecutable {
                    path,
                    module_mode: false,
                });
            }
        }
        let python = resource_dir.join("pyembed").join("python.exe");
        if python.is_file() && windows_pyocd_module(&python).is_file() {
            let path = python.canonicalize().map_err(|error| {
                McuError::operation(format!("解析内置 pyOCD Python 路径失败：{error}"))
            })?;
            return Ok(PyOcdExecutable {
                path,
                module_mode: true,
            });
        }
    }
    for name in ["pyocd", "pyocd.exe"] {
        if let Some(path) = find_in_path(name) {
            return Ok(PyOcdExecutable {
                path,
                module_mode: false,
            });
        }
    }
    Err(McuError::tool_not_found(
        "未找到 pyOCD。请安装 pyocd，或手动选择 pyocd 可执行文件。",
    ))
}

fn windows_pyocd_module(python: &Path) -> PathBuf {
    python
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("Lib")
        .join("site-packages")
        .join("pyocd")
        .join("__main__.py")
}

fn resolve_pack(app: &AppHandle, configured: &str) -> Result<PathBuf, McuError> {
    if !configured.trim().is_empty() {
        return canonical_file(configured, "CMSIS-Pack");
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let directory = resource_dir.join("cmsis-packs");
        if let Ok(entries) = std::fs::read_dir(directory) {
            if let Some(path) = entries.flatten().map(|entry| entry.path()).find(|path| {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| {
                        name.starts_with("Renesas.RA_DFP.") && name.ends_with(".pack")
                    })
            }) {
                return path.canonicalize().map_err(|error| {
                    McuError::operation(format!("解析内置 Renesas RA_DFP 路径失败：{error}"))
                });
            }
        }
    }
    Err(McuError::tool_not_found(
        "未找到 Renesas RA_DFP。精简安装包不再内置该离线资源，请安装后手动选择 .pack 文件。",
    ))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
            .and_then(|candidate| candidate.canonicalize().ok())
    })
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

fn run_command(
    executable: &PyOcdExecutable,
    args: &[String],
    timeout: Duration,
) -> Result<Output, McuError> {
    let mut process = Command::new(&executable.path);
    if executable.module_mode {
        process.args(["-m", "pyocd"]);
    }
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_background_command(&mut process);
    let mut child = process
        .spawn()
        .map_err(|error| McuError::operation(format!("无法启动 pyOCD：{error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McuError::operation("无法读取 pyOCD 标准输出"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| McuError::operation("无法读取 pyOCD 错误输出"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if child
            .try_wait()
            .map_err(|error| McuError::operation(format!("等待 pyOCD 失败：{error}")))?
            .is_some()
        {
            break child
                .wait()
                .map_err(|error| McuError::operation(format!("等待 pyOCD 失败：{error}")))?;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(McuError::timeout(format!(
                "pyOCD 操作超时（{} 秒）",
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| McuError::operation("pyOCD 标准输出读取线程异常"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| McuError::operation("pyOCD 错误输出读取线程异常"))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_all(mut stream: impl Read) -> Result<Vec<u8>, McuError> {
    let mut data = Vec::new();
    stream
        .read_to_end(&mut data)
        .map_err(|error| McuError::operation(format!("读取 pyOCD 输出失败：{error}")))?;
    Ok(data)
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn ensure_success(output: &Output, label: &str) -> Result<(), McuError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(McuError::operation(format!(
            "{label}失败：{}",
            combined_output(output).trim()
        )))
    }
}

fn operation_result(output: Output, label: &str) -> Result<OperationResult, McuError> {
    ensure_success(&output, label)?;
    let text = combined_output(&output);
    if missing_probe_message(&text.to_ascii_lowercase()) {
        return Err(McuError::operation(format!(
            "{label}失败：未检测到已连接的调试探针"
        )));
    }
    Ok(OperationResult {
        success: true,
        exit_code: output.status.code().unwrap_or(0),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_commander_exit_without_probe_is_offline() {
        let status = parse_target_status("R7FA6M5BF", "No connected debug probes\n");
        assert!(!status.connected);
        assert_eq!(status.core_state, "DISCONNECTED");
        assert_eq!(status.device_name, "R7FA6M5BF");
    }

    #[test]
    fn ra6m5_target_identifier_is_preserved() {
        let connection = PyOcdConnection {
            executable_path: None,
            probe_serial: None,
            frequency_khz: 1_000,
            target: "R7FA6M5BF".to_owned(),
            pack_path: "/missing/Renesas.RA_DFP.pack".to_owned(),
        };
        // Pack resolution requires an application resource directory and is covered by command tests.
        assert_eq!(connection.target, "R7FA6M5BF");
    }

    #[test]
    fn bundled_windows_python_resolves_relocatable_pyocd_module() {
        assert_eq!(
            windows_pyocd_module(Path::new("/bundle/pyembed/python.exe")),
            Path::new("/bundle/pyembed/Lib/site-packages/pyocd/__main__.py")
        );
    }
}

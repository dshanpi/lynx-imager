use std::env;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::process::configure_background_command;

use super::commands::{FlashPostAction, McuError, OperationResult, TargetAction, TargetStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const FLASH_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JLinkToolRequest {
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JLinkToolInfo {
    pub available: bool,
    pub executable_path: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JLinkConnection {
    pub executable_path: Option<String>,
    pub probe_serial: Option<String>,
    pub frequency_khz: u32,
    pub device: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JLinkActionRequest {
    pub connection: JLinkConnection,
    pub action: TargetAction,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JLinkFlashRequest {
    pub connection: JLinkConnection,
    pub firmware_path: String,
    pub bin_start_address: Option<u64>,
    pub post_action: FlashPostAction,
}

pub fn tool_info(request: &JLinkToolRequest) -> Result<JLinkToolInfo, McuError> {
    let executable = resolve_executable(request.executable_path.as_deref())?;
    let output = run_command(&executable, &["-?".to_owned()], PROBE_TIMEOUT)?;
    let text = combined_output(&output);
    if !output.status.success() && !text.to_ascii_lowercase().contains("j-link commander") {
        return Err(McuError::operation(format!(
            "检测 SEGGER J-Link Commander 失败：{}",
            text.trim()
        )));
    }
    Ok(JLinkToolInfo {
        available: true,
        executable_path: executable.to_string_lossy().into_owned(),
        version: parse_version(&text),
    })
}

pub fn target_status(connection: &JLinkConnection) -> Result<TargetStatus, McuError> {
    let output = run_script(connection, "connect\nq\n", PROBE_TIMEOUT)?;
    let text = combined_output(&output);
    if missing_probe_or_target(&text) || !output.status.success() {
        return Ok(TargetStatus {
            connected: false,
            device_name: connection.device.trim().to_owned(),
            device_id: String::new(),
            voltage: parse_voltage(&text),
            core_state: "DISCONNECTED".to_owned(),
            output: text,
        });
    }
    Ok(TargetStatus {
        connected: true,
        device_name: connection.device.trim().to_owned(),
        device_id: parse_device_id(&text),
        voltage: parse_voltage(&text),
        core_state: parse_core_state(&text).to_owned(),
        output: text,
    })
}

pub fn target_action(request: &JLinkActionRequest) -> Result<OperationResult, McuError> {
    let script = match request.action {
        TargetAction::ResetRun => "r\ng\nq\n",
        TargetAction::ResetHalt => "r\nh\nq\n",
        TargetAction::Run => "g\nq\n",
        TargetAction::Halt => "h\nq\n",
    };
    let output = run_script(&request.connection, script, ACTION_TIMEOUT)?;
    operation_result(output, "J-Link 目标控制")
}

pub fn flash_internal(request: &JLinkFlashRequest) -> Result<OperationResult, McuError> {
    let firmware = canonical_file(&request.firmware_path, "固件")?;
    let extension = firmware
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let script = flash_script(
        &firmware,
        &extension,
        request.bin_start_address,
        request.post_action,
    )?;
    let output = run_script(&request.connection, &script, FLASH_TIMEOUT)?;
    operation_result(output, "J-Link 内部 Flash 烧写")
}

fn flash_script(
    firmware: &Path,
    extension: &str,
    bin_start_address: Option<u64>,
    post_action: FlashPostAction,
) -> Result<String, McuError> {
    if !matches!(extension, "bin" | "hex" | "elf") {
        return Err(McuError::invalid("J-Link 固件必须是 .bin、.hex 或 .elf"));
    }
    if extension == "bin" && bin_start_address.is_none() {
        return Err(McuError::invalid("JLINK_BIN_ADDRESS_REQUIRED"));
    }
    let firmware = commander_path(firmware)?;
    let mut script = format!("r\nloadfile \"{firmware}\"");
    if let Some(address) = bin_start_address.filter(|_| extension == "bin") {
        script.push_str(&format!(", 0x{address:x}"));
    }
    script.push('\n');
    match post_action {
        FlashPostAction::None => {}
        FlashPostAction::ResetRun => script.push_str("r\ng\n"),
        FlashPostAction::ResetHalt => script.push_str("r\nh\n"),
    }
    script.push_str("q\n");
    Ok(script)
}

fn run_script(
    connection: &JLinkConnection,
    script: &str,
    timeout: Duration,
) -> Result<Output, McuError> {
    let executable = resolve_executable(connection.executable_path.as_deref())?;
    let script_file = CommanderScript::new(script)?;
    let args = connection_args(connection, script_file.path())?;
    run_command(&executable, &args, timeout)
}

fn connection_args(
    connection: &JLinkConnection,
    script_path: &Path,
) -> Result<Vec<String>, McuError> {
    if !(5..=50_000).contains(&connection.frequency_khz) {
        return Err(McuError::invalid("SWD 频率必须在 5 到 50000 kHz 之间"));
    }
    let device = connection.device.trim();
    if device.is_empty()
        || device.len() > 100
        || !device
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
    {
        return Err(McuError::invalid("J-Link Device 格式无效"));
    }
    let mut args = vec![
        "-device".to_owned(),
        device.to_owned(),
        "-if".to_owned(),
        "SWD".to_owned(),
        "-speed".to_owned(),
        connection.frequency_khz.to_string(),
        "-autoconnect".to_owned(),
        "1".to_owned(),
        "-ExitOnError".to_owned(),
        "1".to_owned(),
    ];
    if let Some(serial) = connection
        .probe_serial
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if serial.len() > 32 || !serial.chars().all(|value| value.is_ascii_digit()) {
            return Err(McuError::invalid("J-Link 探针序列号必须是数字"));
        }
        args.extend(["-SelectEmuBySN".to_owned(), serial.to_owned()]);
    }
    args.extend([
        "-CommanderScript".to_owned(),
        script_path.to_string_lossy().into_owned(),
    ]);
    Ok(args)
}

fn resolve_executable(value: Option<&str>) -> Result<PathBuf, McuError> {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        let path = canonical_file(value, "J-Link Commander")?;
        validate_executable_name(&path)?;
        return Ok(path);
    }
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("SEGGER/JLink/JLink.exe"));
        }
    }
    candidates.extend([
        PathBuf::from("/Applications/SEGGER/JLink/JLinkExe"),
        PathBuf::from("/opt/SEGGER/JLink/JLinkExe"),
    ]);
    for candidate in candidates {
        if candidate.is_file() {
            return candidate.canonicalize().map_err(|error| {
                McuError::operation(format!("解析 J-Link Commander 路径失败：{error}"))
            });
        }
    }
    for name in ["JLinkExe", "JLink.exe", "JLinkExe.exe"] {
        if let Some(path) = find_in_path(name) {
            return Ok(path);
        }
    }
    Err(McuError::tool_not_found(
        "未找到 SEGGER J-Link Commander。请安装 J-Link Software and Documentation Pack，或手动选择 JLink.exe/JLinkExe。",
    ))
}

fn validate_executable_name(path: &Path) -> Result<(), McuError> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(name.as_str(), "jlink.exe" | "jlinkexe" | "jlinkexe.exe") {
        Ok(())
    } else {
        Err(McuError::invalid(
            "J-Link 可执行文件名必须是 JLink.exe 或 JLinkExe",
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

fn commander_path(path: &Path) -> Result<String, McuError> {
    let value = path.to_string_lossy();
    if value.contains(['\r', '\n', '"']) {
        return Err(McuError::invalid("J-Link 固件路径包含不安全字符"));
    }
    Ok(value.into_owned())
}

struct CommanderScript {
    path: PathBuf,
}

impl CommanderScript {
    fn new(contents: &str) -> Result<Self, McuError> {
        if contents.contains('\0') {
            return Err(McuError::invalid("CommanderScript 不能包含 NUL 字符"));
        }
        let directory = env::temp_dir();
        for _ in 0..16 {
            let path = directory.join(format!(
                "lynx-jlink-{}-{:016x}.jlink",
                std::process::id(),
                rand::random::<u64>()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    file.write_all(contents.as_bytes()).map_err(|error| {
                        McuError::operation(format!("写入 J-Link CommanderScript 失败：{error}"))
                    })?;
                    file.flush().map_err(|error| {
                        McuError::operation(format!("刷新 J-Link CommanderScript 失败：{error}"))
                    })?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(McuError::operation(format!(
                        "创建 J-Link CommanderScript 失败：{error}"
                    )))
                }
            }
        }
        Err(McuError::operation(
            "无法创建唯一的 J-Link CommanderScript 临时文件",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CommanderScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
        .map_err(|error| McuError::operation(format!("无法启动 J-Link Commander：{error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McuError::operation("无法读取 J-Link 标准输出"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| McuError::operation("无法读取 J-Link 错误输出"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| McuError::operation(format!("等待 J-Link 失败：{error}")))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(McuError::timeout(format!(
                "J-Link 操作超时（{} 秒）",
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| McuError::operation("J-Link 标准输出读取线程异常"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| McuError::operation("J-Link 错误输出读取线程异常"))??;
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
        .map_err(|error| McuError::operation(format!("读取 J-Link 输出失败：{error}")))?;
    Ok(data)
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn missing_probe_or_target(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "cannot connect to j-link",
        "no emulators connected",
        "no j-link device found",
        "cannot connect to target",
        "failed to connect",
        "target connection failed",
    ]
    .iter()
    .any(|message| lower.contains(message))
}

fn operation_result(output: Output, label: &str) -> Result<OperationResult, McuError> {
    let text = combined_output(&output);
    if !output.status.success() || missing_probe_or_target(&text) || has_operation_failure(&text) {
        return Err(McuError::operation(format!("{label}失败：{}", text.trim())));
    }
    Ok(OperationResult {
        success: true,
        exit_code: output.status.code().unwrap_or(0),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn has_operation_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("script processing failed")
        || lower.contains("could not download file")
        || lower.contains("verification failed")
        || lower.contains("failed to erase")
}

fn parse_version(text: &str) -> String {
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains("j-link commander"))
        .map(str::trim)
        .unwrap_or("SEGGER J-Link Commander")
        .to_owned()
}

fn parse_voltage(text: &str) -> Option<f64> {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(index) = lower.find("vtref=") {
            let value = &line[index + 6..];
            let numeric = value
                .trim_start()
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '.')
                .collect::<String>();
            if let Ok(voltage) = numeric.parse::<f64>() {
                return Some(voltage);
            }
        }
    }
    None
}

fn parse_device_id(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower.find("found swd-dp with id").map(|index| {
                line[index + "found swd-dp with id".len()..]
                    .trim_matches([' ', ':'])
                    .to_owned()
            })
        })
        .unwrap_or_default()
}

fn parse_core_state(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("cpu is halted") || lower.contains("pc =") {
        "HALTED"
    } else if lower.contains("cpu is running") {
        "RUNNING"
    } else {
        "CONNECTED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    fn connection() -> JLinkConnection {
        JLinkConnection {
            executable_path: None,
            probe_serial: Some("123456789".to_owned()),
            frequency_khz: 1_000,
            device: "STM32H750ZBT6".to_owned(),
        }
    }

    #[test]
    fn arguments_bind_device_probe_interface_speed_and_script_without_shell() {
        let args = connection_args(&connection(), Path::new("/tmp/test.jlink")).unwrap();
        assert_eq!(
            args,
            vec![
                "-device",
                "STM32H750ZBT6",
                "-if",
                "SWD",
                "-speed",
                "1000",
                "-autoconnect",
                "1",
                "-ExitOnError",
                "1",
                "-SelectEmuBySN",
                "123456789",
                "-CommanderScript",
                "/tmp/test.jlink"
            ]
        );
    }

    #[test]
    fn unsafe_device_and_probe_values_are_rejected() {
        let mut invalid = connection();
        invalid.device = "STM32\nexit".to_owned();
        assert!(connection_args(&invalid, Path::new("/tmp/test")).is_err());
        invalid = connection();
        invalid.probe_serial = Some("123;456".to_owned());
        assert!(connection_args(&invalid, Path::new("/tmp/test")).is_err());
    }

    #[test]
    fn parser_extracts_voltage_id_and_state_and_detects_failure() {
        let text = "VTref=3.300V\nFound SWD-DP with ID 0x6BA02477\nPC = 08000100";
        assert_eq!(parse_voltage(text), Some(3.3));
        assert_eq!(parse_device_id(text), "0x6BA02477");
        assert_eq!(parse_core_state(text), "HALTED");
        assert!(missing_probe_or_target("Cannot connect to target."));
    }

    #[test]
    fn successful_exit_with_flash_failure_is_not_success() {
        let output = Output {
            status: ExitStatus::from_raw(0),
            stdout: b"Could not download file".to_vec(),
            stderr: Vec::new(),
        };
        assert!(operation_result(output, "flash").is_err());
    }

    #[test]
    fn flash_script_requires_bin_address_and_preserves_post_action() {
        let firmware = Path::new("/tmp/application.bin");
        assert_eq!(
            flash_script(firmware, "bin", None, FlashPostAction::None)
                .unwrap_err()
                .message,
            "JLINK_BIN_ADDRESS_REQUIRED"
        );
        assert_eq!(
            flash_script(
                firmware,
                "bin",
                Some(0x0800_0000),
                FlashPostAction::ResetRun
            )
            .unwrap(),
            "r\nloadfile \"/tmp/application.bin\", 0x8000000\nr\ng\nq\n"
        );
    }
}

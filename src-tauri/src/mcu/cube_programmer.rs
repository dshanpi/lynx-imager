use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use goblin::elf::program_header::PT_LOAD;

use crate::process::configure_background_command;

use super::commands::{
    ConnectMode, FlashPostAction, FlashRequest, McuError, OperationResult, StLinkConnection,
    StLinkProbe, TargetAction, TargetActionRequest, TargetStatus, ToolInfo, ToolRequest,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const FLASH_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MIN_EXTERNAL_ADDRESS: u64 = 0x6000_0000;
const MAX_EXTERNAL_ADDRESS_EXCLUSIVE: u64 = 0xE000_0000;

pub(super) fn tool_info(request: &ToolRequest) -> Result<ToolInfo, McuError> {
    let executable = resolve_executable(request.executable_path.as_deref())?;
    let output = run_command(&executable, &["--version".to_owned()], PROBE_TIMEOUT)?;
    let version = combined_output(&output).trim().to_owned();
    Ok(ToolInfo {
        available: output.status.success(),
        executable_path: executable.to_string_lossy().into_owned(),
        version,
    })
}

pub(super) fn list_probes(request: &ToolRequest) -> Result<Vec<StLinkProbe>, McuError> {
    let executable = resolve_executable(request.executable_path.as_deref())?;
    let output = run_command(&executable, &["-l".to_owned()], PROBE_TIMEOUT)?;
    ensure_success(&output, "枚举 ST-Link")?;
    Ok(parse_probe_list(&combined_output(&output)))
}

pub(super) fn target_status(connection: &StLinkConnection) -> Result<TargetStatus, McuError> {
    let executable = resolve_executable(connection.executable_path.as_deref())?;
    let mut args = connect_args(connection)?;
    args.push("-score".to_owned());
    let output = run_command(&executable, &args, ACTION_TIMEOUT)?;
    ensure_success(&output, "读取目标状态")?;
    let text = combined_output(&output);
    Ok(parse_target_status(&text))
}

pub(super) fn target_action(request: &TargetActionRequest) -> Result<OperationResult, McuError> {
    let executable = resolve_executable(request.connection.executable_path.as_deref())?;
    let mut connection = request.connection.clone();
    connection.connect_mode = match request.action {
        TargetAction::ResetHalt => ConnectMode::Normal,
        _ => ConnectMode::HotPlug,
    };
    let mut args = connect_args(&connection)?;
    match request.action {
        TargetAction::ResetRun => args.extend(["-rst".to_owned(), "-g".to_owned()]),
        TargetAction::ResetHalt => args.extend(["-rst".to_owned(), "-halt".to_owned()]),
        TargetAction::Run => args.push("-g".to_owned()),
        TargetAction::Halt => args.push("-halt".to_owned()),
    }
    let output = run_command(&executable, &args, ACTION_TIMEOUT)?;
    operation_result(output, "控制目标")
}

pub(super) fn flash_ospi(request: &FlashRequest) -> Result<OperationResult, McuError> {
    let executable = resolve_executable(request.connection.executable_path.as_deref())?;
    let firmware = canonical_file(&request.firmware_path, "固件")?;
    let loader = canonical_file(&request.external_loader_path, "External Loader")?;
    if !extension_eq(&loader, "stldr") {
        return Err(McuError::invalid("External Loader 必须是 .stldr 文件"));
    }
    validate_external_range(
        request.external_address_start,
        request.external_address_size,
    )?;
    validate_firmware_addresses(
        &firmware,
        request.external_address_start,
        request.external_address_size,
        request.bin_start_address,
    )?;

    let args = build_flash_args(request, &firmware, &loader)?;
    let output = run_command(&executable, &args, FLASH_TIMEOUT)?;
    operation_result(output, "烧写并校验 OSPI NOR")
}

fn build_flash_args(
    request: &FlashRequest,
    firmware: &Path,
    loader: &Path,
) -> Result<Vec<String>, McuError> {
    let mut connection = request.connection.clone();
    connection.connect_mode = ConnectMode::Normal;
    let mut args = connect_args(&connection)?;
    args.extend(["-w".to_owned(), firmware.to_string_lossy().into_owned()]);
    if extension_eq(firmware, "bin") {
        let address = request
            .bin_start_address
            .ok_or_else(|| McuError::invalid("BIN 固件必须设置 OSPI 起始地址"))?;
        args.push(format!("0x{address:08X}"));
    }
    args.extend([
        "-v".to_owned(),
        "-el".to_owned(),
        loader.to_string_lossy().into_owned(),
    ]);
    match request.post_action {
        FlashPostAction::None => {}
        FlashPostAction::ResetRun => args.extend(["-rst".to_owned(), "-g".to_owned()]),
        FlashPostAction::ResetHalt => args.extend(["-rst".to_owned(), "-halt".to_owned()]),
    }
    Ok(args)
}

fn resolve_executable(configured: Option<&str>) -> Result<PathBuf, McuError> {
    if let Some(value) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        let path = canonical_file(value, "STM32CubeProgrammer CLI")?;
        validate_executable_name(&path)?;
        return Ok(path);
    }

    for name in [
        "STM32_Programmer_CLI",
        "STM32_Programmer_CLI.exe",
        "STM32_Programmer.sh",
    ] {
        if let Some(path) = find_in_path(name) {
            return Ok(path);
        }
    }

    for candidate in default_candidates() {
        if candidate.is_file() {
            return candidate.canonicalize().map_err(|error| {
                McuError::operation(format!("无法解析 STM32CubeProgrammer 路径：{error}"))
            });
        }
    }

    Err(McuError::tool_not_found(
        "未找到 STM32_Programmer_CLI。请安装 STM32CubeProgrammer，或在 MCU 工作台中选择 CLI 路径。",
    ))
}

fn validate_executable_name(path: &Path) -> Result<(), McuError> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "stm32_programmer_cli" | "stm32_programmer_cli.exe" | "stm32_programmer.sh"
    ) {
        Ok(())
    } else {
        Err(McuError::invalid(
            "CLI 文件名必须是 STM32_Programmer_CLI、STM32_Programmer_CLI.exe 或 STM32_Programmer.sh",
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

fn default_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/st/stm32cubeprogrammer/bin/STM32_Programmer_CLI"),
        PathBuf::from("/Applications/STMicroelectronics/STM32Cube/STM32CubeProgrammer/STM32CubeProgrammer.app/Contents/MacOs/bin/STM32_Programmer_CLI"),
    ];
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env::var_os(key) {
            candidates.push(PathBuf::from(root).join(
                "STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI.exe",
            ));
        }
    }
    if let Some(home) = env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join("STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI"),
        );
    }
    candidates
}

fn connect_args(connection: &StLinkConnection) -> Result<Vec<String>, McuError> {
    if !(5..=50_000).contains(&connection.frequency_khz) {
        return Err(McuError::invalid("SWD 频率必须在 5 kHz 到 50000 kHz 之间"));
    }
    let mut options = vec![
        "port=SWD".to_owned(),
        format!("freq={}", connection.frequency_khz),
        format!(
            "mode={}",
            match connection.connect_mode {
                ConnectMode::Normal => "NORMAL",
                ConnectMode::UnderReset => "UR",
                ConnectMode::HotPlug => "HOTPLUG",
            }
        ),
    ];
    if let Some(serial) = connection
        .probe_serial
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if serial.len() > 64 || !serial.chars().all(|value| value.is_ascii_alphanumeric()) {
            return Err(McuError::invalid("ST-Link 序列号格式无效"));
        }
        options.push(format!("sn={serial}"));
    } else if let Some(index) = connection.probe_index {
        options.push(format!("index={index}"));
    }
    let mut args = vec!["-c".to_owned()];
    args.extend(options);
    Ok(args)
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

pub(super) fn validate_external_range(start: u64, size: u64) -> Result<(), McuError> {
    let end = start
        .checked_add(size)
        .ok_or_else(|| McuError::invalid("OSPI 地址范围溢出"))?;
    if size == 0 || start < MIN_EXTERNAL_ADDRESS || end > MAX_EXTERNAL_ADDRESS_EXCLUSIVE {
        return Err(McuError::invalid(format!(
            "OSPI 范围必须位于 0x{MIN_EXTERNAL_ADDRESS:08X}..0x{MAX_EXTERNAL_ADDRESS_EXCLUSIVE:08X}，且容量不能为 0"
        )));
    }
    Ok(())
}

fn validate_span(address: u64, size: u64, start: u64, range_size: u64) -> Result<(), McuError> {
    let end = address
        .checked_add(size)
        .ok_or_else(|| McuError::invalid("固件地址溢出"))?;
    let range_end = start
        .checked_add(range_size)
        .ok_or_else(|| McuError::invalid("OSPI 地址范围溢出"))?;
    if size == 0 || address < start || end > range_end {
        return Err(McuError::invalid(format!(
            "固件数据 0x{address:08X}..0x{end:08X} 超出 OSPI 范围 0x{start:08X}..0x{range_end:08X}"
        )));
    }
    Ok(())
}

pub(super) fn validate_firmware_addresses(
    path: &Path,
    start: u64,
    size: u64,
    bin_start: Option<u64>,
) -> Result<(), McuError> {
    let data =
        fs::read(path).map_err(|error| McuError::invalid(format!("读取固件失败：{error}")))?;
    if data.is_empty() {
        return Err(McuError::invalid("固件文件为空"));
    }
    if extension_eq(path, "bin") {
        return validate_span(
            bin_start.ok_or_else(|| McuError::invalid("BIN 固件必须设置 OSPI 起始地址"))?,
            data.len() as u64,
            start,
            size,
        );
    }
    if extension_eq(path, "hex") {
        return validate_hex(&data, start, size);
    }
    if extension_eq(path, "elf") || extension_eq(path, "axf") {
        let elf = goblin::elf::Elf::parse(&data)
            .map_err(|error| McuError::invalid(format!("ELF 解析失败：{error}")))?;
        let mut count = 0;
        for header in elf
            .program_headers
            .iter()
            .filter(|header| header.p_type == PT_LOAD && header.p_filesz > 0)
        {
            let address = if header.p_paddr != 0 {
                header.p_paddr
            } else {
                header.p_vaddr
            };
            validate_span(address, header.p_filesz, start, size)?;
            count += 1;
        }
        return if count == 0 {
            Err(McuError::invalid("ELF 中没有可烧写的 LOAD 段"))
        } else {
            Ok(())
        };
    }
    Err(McuError::invalid("仅支持 BIN、HEX、ELF 和 AXF 固件"))
}

fn validate_hex(data: &[u8], start: u64, size: u64) -> Result<(), McuError> {
    let text =
        std::str::from_utf8(data).map_err(|_| McuError::invalid("HEX 文件不是有效 UTF-8 文本"))?;
    let mut upper = 0u64;
    let mut data_records = 0usize;
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let bytes = decode_hex_record(line).map_err(|message| {
            McuError::invalid(format!("HEX 第 {} 行：{message}", line_index + 1))
        })?;
        let length = bytes[0] as usize;
        let offset = u16::from_be_bytes([bytes[1], bytes[2]]) as u64;
        match bytes[3] {
            0x00 => {
                validate_span(upper + offset, length as u64, start, size)?;
                data_records += 1;
            }
            0x01 => break,
            0x02 => upper = u16::from_be_bytes([bytes[4], bytes[5]]) as u64 * 16,
            0x04 => upper = (u16::from_be_bytes([bytes[4], bytes[5]]) as u64) << 16,
            _ => {}
        }
    }
    if data_records == 0 {
        Err(McuError::invalid("HEX 中没有数据记录"))
    } else {
        Ok(())
    }
}

fn decode_hex_record(line: &str) -> Result<Vec<u8>, &'static str> {
    let payload = line.strip_prefix(':').ok_or("记录必须以冒号开头")?;
    if payload.len() < 10 || payload.len() % 2 != 0 {
        return Err("记录长度无效");
    }
    let bytes = (0..payload.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&payload[index..index + 2], 16).map_err(|_| "存在非十六进制字符")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let length = bytes[0] as usize;
    if bytes.len() != length + 5 {
        return Err("数据长度字段不匹配");
    }
    if bytes
        .iter()
        .fold(0u8, |sum, value| sum.wrapping_add(*value))
        != 0
    {
        return Err("校验和错误");
    }
    Ok(bytes)
}

fn run_command(executable: &Path, args: &[String], timeout: Duration) -> Result<Output, McuError> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| McuError::operation(format!("无法启动 STM32CubeProgrammer：{error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McuError::operation("无法读取 CLI 标准输出"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| McuError::operation("无法读取 CLI 错误输出"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|error| McuError::operation(format!("等待 CLI 失败：{error}")))?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(McuError::timeout(format!(
                    "STM32CubeProgrammer 超时（{} 秒）",
                    timeout.as_secs()
                )));
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| McuError::operation("CLI 输出读取线程异常"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| McuError::operation("CLI 错误读取线程异常"))??;
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
        .map_err(|error| McuError::operation(format!("读取 CLI 输出失败：{error}")))?;
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

fn parse_probe_list(text: &str) -> Vec<StLinkProbe> {
    let mut probes = Vec::new();
    let mut current: Option<StLinkProbe> = None;
    for line in text.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        if lower.contains("st-link probe") || lower.contains("stlink probe") {
            if let Some(probe) = current.take() {
                probes.push(probe);
            }
            current = Some(StLinkProbe {
                index: probes.len() as u32,
                serial_number: String::new(),
                firmware_version: String::new(),
                board_name: String::new(),
            });
            continue;
        }
        let Some(probe) = current.as_mut() else {
            continue;
        };
        if let Some(value) = line_value(line, &["ST-LINK SN", "STLINK SN", "Serial Number"]) {
            probe.serial_number = value;
        } else if let Some(value) =
            line_value(line, &["ST-LINK FW", "STLINK FW", "Firmware version"])
        {
            probe.firmware_version = value;
        } else if let Some(value) = line_value(line, &["Board", "Board Name"]) {
            probe.board_name = value;
        }
    }
    if let Some(probe) = current {
        probes.push(probe);
    }
    probes
}

fn parse_target_status(text: &str) -> TargetStatus {
    let voltage_text = first_value(text, &["Voltage", "Target voltage"]);
    let voltage = voltage_text.as_deref().and_then(|value| {
        value
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .find(|part| !part.is_empty())
            .and_then(|part| part.parse().ok())
    });
    TargetStatus {
        connected: true,
        device_name: first_value(text, &["Device name", "Device Name"]).unwrap_or_default(),
        device_id: first_value(text, &["Device ID", "Device Id"]).unwrap_or_default(),
        voltage,
        core_state: first_value(text, &["Core state", "Core State"])
            .unwrap_or_else(|| "UNKNOWN".to_owned()),
        output: text.to_owned(),
    }
}

fn first_value(text: &str, keys: &[&str]) -> Option<String> {
    text.lines().find_map(|line| line_value(line.trim(), keys))
}

fn line_value(line: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        if line
            .to_ascii_lowercase()
            .starts_with(&key.to_ascii_lowercase())
        {
            line.split_once([':', '='])
                .map(|(_, value)| value.trim().to_owned())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_internal_flash_bin_address() {
        let error = validate_span(0x0800_0000, 1024, 0x9000_0000, 0x0400_0000).unwrap_err();
        assert_eq!(error.name, "McuInvalidConfiguration");
    }

    #[test]
    fn accepts_bin_fully_inside_ospi_range() {
        validate_span(0x9000_1000, 4096, 0x9000_0000, 0x0400_0000).unwrap();
    }

    #[test]
    fn validates_hex_checksum_and_external_address() {
        let hex = b":0200000490006A\n:0400000001020304F2\n:00000001FF\n";
        validate_hex(hex, 0x9000_0000, 0x1000).unwrap();
    }

    #[test]
    fn rejects_hex_internal_address() {
        let hex = b":020000040800F2\n:0400000001020304F2\n:00000001FF\n";
        assert!(validate_hex(hex, 0x9000_0000, 0x1000).is_err());
    }

    #[test]
    fn builds_exact_connection_argument_boundaries() {
        let args = connect_args(&StLinkConnection {
            executable_path: None,
            probe_serial: Some("55FF1234".to_owned()),
            probe_index: Some(8),
            frequency_khz: 4000,
            connect_mode: ConnectMode::UnderReset,
        })
        .unwrap();
        assert_eq!(
            args,
            ["-c", "port=SWD", "freq=4000", "mode=UR", "sn=55FF1234"]
        );
        assert!(!args.iter().any(|arg| arg.contains("index=")));
    }

    #[test]
    fn parses_probe_blocks() {
        let probes = parse_probe_list("===== ST-LINK Probe 0 =====\nST-LINK SN  : 55FFAA\nST-LINK FW  : V2J45S7\nBoard       : --\n");
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].serial_number, "55FFAA");
        assert_eq!(probes[0].firmware_version, "V2J45S7");
    }

    #[test]
    fn builds_exact_ospi_bin_flash_arguments() {
        let request = FlashRequest {
            connection: StLinkConnection {
                executable_path: None,
                probe_serial: Some("55FF1234".to_owned()),
                probe_index: None,
                frequency_khz: 4000,
                connect_mode: ConnectMode::HotPlug,
            },
            firmware_path: "ignored.bin".to_owned(),
            external_loader_path: "ignored.stldr".to_owned(),
            external_address_start: 0x9000_0000,
            external_address_size: 0x0400_0000,
            bin_start_address: Some(0x9000_0000),
            post_action: FlashPostAction::ResetRun,
        };
        let args = build_flash_args(
            &request,
            Path::new("firmware.bin"),
            Path::new("board-loader.stldr"),
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "-c",
                "port=SWD",
                "freq=4000",
                "mode=NORMAL",
                "sn=55FF1234",
                "-w",
                "firmware.bin",
                "0x90000000",
                "-v",
                "-el",
                "board-loader.stldr",
                "-rst",
                "-g",
            ]
        );
    }
}

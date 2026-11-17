use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{command, AppHandle, Manager, State};

use crate::serial::{
    spawn_read_thread, SerialAuditContext, SerialCapture, SerialCaptureChunk, SerialCaptureStatus,
    SerialPortConfiguration, SerialPortHandle, SerialReadDelivery, SerialState,
};

/// Information about an available serial port.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: String,
    pub description: String,
    pub serial_number: String,
    pub interface_number: Option<u8>,
}

/// Stable serial-port identity used across USB disconnect/re-enumeration.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialIdentity {
    /// Last known OS node, also used to disambiguate multi-port USB adapters.
    pub port: String,
    pub vid: u16,
    pub pid: u16,
    pub serial_number: String,
    #[serde(default)]
    pub interface_number: Option<u8>,
}

fn parse_data_bits(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn parse_stop_bits(n: u8) -> StopBits {
    if n >= 2 {
        StopBits::Two
    } else {
        StopBits::One
    }
}

fn parse_parity(s: &str) -> Parity {
    match s.to_lowercase().as_str() {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    }
}

fn parse_flow_control(s: &str) -> FlowControl {
    match s.to_lowercase().as_str() {
        "rts_cts" | "hardware" => FlowControl::Hardware,
        "xon_xoff" | "software" => FlowControl::Software,
        _ => FlowControl::None,
    }
}

fn serial_configuration(
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: &str,
    flow_control: &str,
) -> SerialPortConfiguration {
    SerialPortConfiguration {
        baud_rate,
        data_bits: match data_bits {
            5..=7 => data_bits,
            _ => 8,
        },
        stop_bits: if stop_bits >= 2 { 2 } else { 1 },
        parity: match parity.to_ascii_lowercase().as_str() {
            "odd" => "odd",
            "even" => "even",
            _ => "none",
        }
        .to_owned(),
        flow_control: match flow_control.to_ascii_lowercase().as_str() {
            "rts_cts" | "hardware" => "hardware",
            "xon_xoff" | "software" => "software",
            _ => "none",
        }
        .to_owned(),
    }
}

/// List all available serial ports.
#[command]
pub fn serial_list_ports(app_handle: AppHandle) -> Result<Vec<SerialPortInfo>, String> {
    crate::hardware_safety::ensure_idle(&app_handle, "扫描 USB 串口")?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("串口", "USB 端口扫描", "serial")?;
    Ok(list_ports_core())
}

/// Platform-neutral port enumeration used by commands and workflows.
pub fn list_ports_core() -> Vec<SerialPortInfo> {
    match serialport::available_ports() {
        Ok(ports) => {
            log::debug!("serial_list_ports: found {} port(s)", ports.len());
            let mut ports: Vec<_> = ports
                .into_iter()
                .map(|p| {
                    let (vid, pid, manufacturer, description, serial_number, interface_number) =
                        match p.port_type {
                            serialport::SerialPortType::UsbPort(info) => (
                                info.vid,
                                info.pid,
                                info.manufacturer.unwrap_or_default(),
                                info.product.unwrap_or_default(),
                                info.serial_number.unwrap_or_default(),
                                info.interface,
                            ),
                            _ => (
                                0u16,
                                0u16,
                                String::new(),
                                String::new(),
                                String::new(),
                                None,
                            ),
                        };
                    log::trace!(
                        "serial port: {} (vid={:04x}, pid={:04x})",
                        p.port_name,
                        vid,
                        pid
                    );
                    SerialPortInfo {
                        interface_number: interface_number
                            .or_else(|| detect_interface_number(&p.port_name)),
                        name: p.port_name,
                        vid,
                        pid,
                        manufacturer,
                        description,
                        serial_number,
                    }
                })
                .collect();
            ports.sort_by(|left, right| {
                (
                    left.vid,
                    left.pid,
                    &left.serial_number,
                    left.interface_number,
                    &left.name,
                )
                    .cmp(&(
                        right.vid,
                        right.pid,
                        &right.serial_number,
                        right.interface_number,
                        &right.name,
                    ))
            });
            ports
        }
        Err(e) => {
            log::error!("serial_list_ports failed: {}", e);
            vec![]
        }
    }
}

/// Resolve a stable identity to the current OS port name.
///
/// USB serial number has the highest priority. VID/PID-only bindings are
/// accepted only when exactly one matching device exists, preventing a board
/// from silently binding to the wrong adapter.
pub fn resolve_port_core(identity: &SerialIdentity) -> Result<SerialPortInfo, String> {
    resolve_port_from(identity, list_ports_core())
}

fn resolve_port_from(
    identity: &SerialIdentity,
    ports: Vec<SerialPortInfo>,
) -> Result<SerialPortInfo, String> {
    if !identity.serial_number.trim().is_empty() {
        let matches: Vec<_> = ports
            .into_iter()
            .filter(|port| {
                port.serial_number == identity.serial_number
                    && (identity.vid == 0 || port.vid == identity.vid)
                    && (identity.pid == 0 || port.pid == identity.pid)
                    && identity
                        .interface_number
                        .is_none_or(|number| port.interface_number == Some(number))
            })
            .collect();
        if matches.len() > 1 && !identity.port.trim().is_empty() {
            if let Some(exact) = matches.iter().find(|port| port.name == identity.port) {
                return Ok(exact.clone());
            }
        }
        return match matches.len() {
            0 => {
                Err("SERIAL_NOT_FOUND:configured USB serial/interface did not enumerate".to_owned())
            }
            1 => Ok(matches.into_iter().next().expect("one port")),
            count => Err(format!(
                "SERIAL_AMBIGUOUS:{count} ports share serial {}; select and save the exact OS node",
                identity.serial_number
            )),
        };
    }

    if identity.vid != 0 || identity.pid != 0 {
        let matches: Vec<_> = ports
            .into_iter()
            .filter(|port| {
                (identity.vid == 0 || port.vid == identity.vid)
                    && (identity.pid == 0 || port.pid == identity.pid)
                    && identity
                        .interface_number
                        .is_none_or(|number| port.interface_number == Some(number))
            })
            .collect();
        if matches.len() > 1 && !identity.port.trim().is_empty() {
            if let Some(exact) = matches.iter().find(|port| port.name == identity.port) {
                return Ok(exact.clone());
            }
        }
        return match matches.len() {
            0 => Err("SERIAL_NOT_FOUND:VID/PID did not enumerate".to_owned()),
            1 => Ok(matches.into_iter().next().expect("one port")),
            count => Err(format!(
                "SERIAL_AMBIGUOUS:{count} ports match VID={:04x},PID={:04x}; configure a serial number",
                identity.vid, identity.pid
            )),
        };
    }

    if identity.port.trim().is_empty() {
        return Err("SERIAL_IDENTITY_EMPTY".to_owned());
    }
    if let Some(port) = ports.into_iter().find(|port| port.name == identity.port) {
        return Ok(port);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if std::fs::metadata(&identity.port)
            .is_ok_and(|metadata| metadata.file_type().is_char_device())
        {
            return Ok(SerialPortInfo {
                name: identity.port.clone(),
                vid: 0,
                pid: 0,
                manufacturer: String::new(),
                description: "Configured character-device serial port".to_owned(),
                serial_number: String::new(),
                interface_number: None,
            });
        }
    }
    Err(format!("SERIAL_NOT_FOUND:{}", identity.port))
}

#[cfg(target_os = "linux")]
fn detect_interface_number(port_name: &str) -> Option<u8> {
    let name = std::path::Path::new(port_name).file_name()?;
    let mut path = std::fs::canonicalize(std::path::Path::new("/sys/class/tty").join(name)).ok()?;
    loop {
        let value = path.join("bInterfaceNumber");
        if let Ok(text) = std::fs::read_to_string(value) {
            return u8::from_str_radix(text.trim(), 16).ok();
        }
        if !path.pop() {
            return None;
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn detect_interface_number(_port_name: &str) -> Option<u8> {
    None
}

/// Resolve a device binding without opening the port.
#[command]
pub fn serial_resolve_port(identity: SerialIdentity) -> Result<SerialPortInfo, String> {
    resolve_port_core(&identity)
}

/// Release a matching port if it is currently owned by this application.
///
/// An already-closed or temporarily absent port is a successful no-op. A port
/// held by another application remains detectable later when opening/flashing.
#[command]
pub fn serial_release_matching(
    identity: SerialIdentity,
    app_handle: AppHandle,
    state: State<'_, Arc<SerialState>>,
) -> Result<Option<String>, String> {
    crate::hardware_safety::ensure_idle(&app_handle, "释放 USB 串口")?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("串口", "释放 USB 串口", identity.port.clone())?;
    let resolved = match resolve_port_core(&identity) {
        Ok(port) => Some(port.name),
        Err(error) if error.starts_with("SERIAL_NOT_FOUND:") => {
            if identity.port.is_empty() {
                None
            } else {
                Some(identity.port)
            }
        }
        Err(error) => return Err(error),
    };
    let Some(port) = resolved else {
        return Ok(None);
    };
    if serial_is_open_core(&state, &port) {
        serial_close_core(&state, &port)?;
        Ok(Some(port))
    } else {
        Ok(None)
    }
}

/// Wait for the bound USB serial adapter to re-enumerate and open it.
#[command]
#[allow(clippy::too_many_arguments)]
pub async fn serial_wait_and_open(
    identity: SerialIdentity,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: String,
    flow_control: String,
    timeout_ms: u64,
    state: State<'_, Arc<SerialState>>,
    app_handle: AppHandle,
    audit_context: Option<SerialAuditContext>,
) -> Result<SerialPortInfo, String> {
    crate::hardware_safety::ensure_idle(&app_handle, "打开 USB 串口")?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("串口", "打开 USB 串口", identity.port.clone())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms.clamp(100, 120_000));
        let resolved = loop {
            match resolve_port_core(&identity) {
                Ok(port) => break port,
                Err(error) if error.starts_with("SERIAL_NOT_FOUND:") => {
                    if std::time::Instant::now() >= deadline {
                        return Err(format!("SERIAL_ENUM_TIMEOUT:{error}"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                Err(error) => return Err(error),
            }
        };
        let requested =
            serial_configuration(baud_rate, data_bits, stop_bits, &parity, &flow_control);
        let matches_open_configuration = state
            .ports
            .lock()
            .map_err(|error| format!("Lock error: {error}"))?
            .get(&resolved.name)
            .is_some_and(|handle| {
                !handle.disconnected.load(Ordering::Relaxed) && handle.configuration == requested
            });
        if !matches_open_configuration {
            if serial_is_open_core(&state, &resolved.name) {
                log::info!(
                    "serial_wait_and_open: reopening {} because configuration changed to {} baud",
                    resolved.name,
                    baud_rate
                );
                serial_close_core(&state, &resolved.name)?;
            }
            serial_open_core(
                &state,
                app_handle,
                resolved.name.clone(),
                baud_rate,
                data_bits,
                stop_bits,
                parity,
                flow_control,
                audit_context,
            )?;
        }
        Ok(resolved)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a serial port with the given configuration.
#[command]
#[allow(clippy::too_many_arguments)]
pub fn serial_open(
    port: String,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: String,
    flow_control: String,
    state: State<'_, Arc<SerialState>>,
    app_handle: AppHandle,
    audit_context: Option<SerialAuditContext>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app_handle, "打开 USB 串口")?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("串口", "打开 USB 串口", port.clone())?;
    serial_open_core(
        &state,
        app_handle,
        port,
        baud_rate,
        data_bits,
        stop_bits,
        parity,
        flow_control,
        audit_context,
    )
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
/// `app` 用于读线程 emit `serial-data-received` 事件——HTTP 发起的 open 同样会推送到前端。
#[allow(clippy::too_many_arguments)]
pub fn serial_open_core(
    state: &SerialState,
    app: AppHandle,
    port: String,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: String,
    flow_control: String,
    audit_context: Option<SerialAuditContext>,
) -> Result<(), String> {
    if let Some(context) = audit_context.as_ref() {
        validate_audit_context(context)?;
    }
    // A read thread leaves its handle marked disconnected so callers can
    // observe the transition. Reopening the same OS path must retire that
    // stale handle first; otherwise every reconnect attempt reports
    // "already open" forever even though serial_is_open_core returned false.
    {
        let mut ports = state
            .ports
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(handle) = ports.get(&port) {
            if !handle.disconnected.load(Ordering::Relaxed) {
                return Err(format!("Port {} is already open", port));
            }
            log::info!("Retiring disconnected serial handle before reopen: {port}");
            ports.remove(&port);
        }
    }

    log::info!(
        "serial_open: port={}, baud={}, data={}, stop={}, parity={}, flow={}",
        port,
        baud_rate,
        data_bits,
        stop_bits,
        parity,
        flow_control
    );

    let configuration =
        serial_configuration(baud_rate, data_bits, stop_bits, &parity, &flow_control);
    let port_handle = serialport::new(&port, baud_rate)
        .data_bits(parse_data_bits(data_bits))
        .stop_bits(parse_stop_bits(stop_bits))
        .parity(parse_parity(&parity))
        .flow_control(parse_flow_control(&flow_control))
        .timeout(std::time::Duration::from_millis(2))
        .open()
        .map_err(|e| {
            log::error!("serial_open failed for {}: {}", port, e);
            let msg = e.to_string().to_lowercase();
            if msg.contains("access")
                || msg.contains("denied")
                || msg.contains("refused")
                || msg.contains("permission")
                || msg.contains("拒绝")
            {
                format!("PORT_BUSY:{}", port)
            } else if msg.contains("not found")
                || msg.contains("不存在")
                || msg.contains("does not exist")
            {
                format!("PORT_GONE:{}", port)
            } else {
                format!("PORT_ERROR:{}:{}", port, e)
            }
        })?;

    let port_name = port.clone();
    let reader = port_handle.try_clone().map_err(|error| {
        log::error!("serial clone failed for {}: {}", port, error);
        format!("PORT_CLONE_ERROR:{}:{}", port, error)
    })?;
    let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let disconnected = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let read_buffer = Arc::new(std::sync::Mutex::new(VecDeque::<u8>::new()));
    let capture = SerialCapture::create(&app)?;

    let read_handle = spawn_read_thread(
        port_name.clone(),
        reader,
        stop_flag.clone(),
        disconnected.clone(),
        SerialReadDelivery {
            read_buffer: read_buffer.clone(),
            app_handle: app.clone(),
            audit_context: audit_context.clone(),
            capture: capture.clone(),
        },
    );

    let handle = SerialPortHandle {
        writer: std::sync::Mutex::new(port_handle),
        configuration,
        read_thread: Some(read_handle),
        stop_flag,
        disconnected,
        read_buffer,
        capture,
        audit_app: Some(app.clone()),
        audit_context: audit_context.clone(),
    };

    let mut ports = state
        .ports
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    ports.insert(port_name, handle);
    if let Some(context) = audit_context.as_ref() {
        publish_serial_connected(&app, context, &port, baud_rate);
    }
    log::info!("Serial port {} opened", port);
    Ok(())
}

/// Close an open serial port.
#[command]
pub fn serial_close(
    port: String,
    app_handle: AppHandle,
    state: State<'_, Arc<SerialState>>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app_handle, "关闭 USB 串口")?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("串口", "关闭 USB 串口", port.clone())?;
    serial_close_core(&state, &port)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn serial_close_core(state: &SerialState, port: &str) -> Result<(), String> {
    let mut ports = state
        .ports
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(mut handle) = ports.remove(port) {
        let audit = (!handle.disconnected.load(Ordering::Relaxed))
            .then(|| handle.audit_app.clone().zip(handle.audit_context.clone()))
            .flatten();
        handle
            .stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = handle.read_thread.take() {
            let _ = thread.join();
        }
        drop(ports);
        if let Some((app, context)) = audit {
            publish_serial_disconnected(&app, &context, port, "explicit_close");
        }
        log::info!("Serial port {} closed", port);
        Ok(())
    } else {
        Err(format!("Port {} is not open", port))
    }
}

/// Close every serial handle owned by this LYNX process. Hardware workflows
/// call this only after acquiring the process-wide hardware activity guard, so
/// frontend reconciliation cannot reopen a port until the workflow finishes.
#[allow(dead_code)]
pub fn serial_close_all_core(state: &SerialState) -> Result<Vec<String>, String> {
    let mut ports = state
        .ports
        .lock()
        .map_err(|error| format!("Lock error: {error}"))?;
    let mut closed = ports.keys().cloned().collect::<Vec<_>>();
    closed.sort();
    let audits = ports
        .iter()
        .filter_map(|(port, handle)| {
            if handle.disconnected.load(Ordering::Relaxed) {
                return None;
            }
            Some((
                port.clone(),
                handle.audit_app.clone()?,
                handle.audit_context.clone()?,
            ))
        })
        .collect::<Vec<_>>();
    ports.clear();
    drop(ports);
    for (port, app, context) in audits {
        publish_serial_disconnected(&app, &context, &port, "close_all");
    }
    Ok(closed)
}

/// Write data to an open serial port.
#[command]
pub fn serial_write(
    port: String,
    data: Vec<u8>,
    state: State<'_, Arc<SerialState>>,
) -> Result<(), String> {
    serial_write_core(&state, &port, &data)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn serial_write_core(state: &SerialState, port: &str, data: &[u8]) -> Result<(), String> {
    serial_write_impl(state, port, data, None)
}

/// Write through an existing shared serial handle while attributing this
/// specific transmission to the calling task. This avoids replacing the
/// long-lived UI/session audit context when a Supervisor flow reuses a port.
pub fn serial_write_audited_core(
    state: &SerialState,
    port: &str,
    data: &[u8],
    app: &AppHandle,
    audit_context: &SerialAuditContext,
) -> Result<(), String> {
    validate_audit_context(audit_context)?;
    serial_write_impl(
        state,
        port,
        data,
        Some((app.clone(), audit_context.clone())),
    )
}

fn serial_write_impl(
    state: &SerialState,
    port: &str,
    data: &[u8],
    audit_override: Option<(AppHandle, SerialAuditContext)>,
) -> Result<(), String> {
    let ports = state
        .ports
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(handle) = ports.get(port) {
        let audit =
            audit_override.or_else(|| handle.audit_app.clone().zip(handle.audit_context.clone()));
        let mut port_guard = handle
            .writer
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        port_guard.write_all(data).map_err(|e| {
            log::error!("serial_write failed for {}: {}", port, e);
            format!("Write error: {}", e)
        })?;
        port_guard
            .flush()
            .map_err(|e| format!("Flush error: {}", e))?;
        drop(port_guard);
        drop(ports);
        if let Some((app, context)) = audit {
            publish_serial_tx(&app, &context, port, data.len());
        }
        Ok(())
    } else {
        Err(format!("Port {} is not open", port))
    }
}

fn publish_serial_connected(
    app: &AppHandle,
    context: &SerialAuditContext,
    port: &str,
    baud_rate: u32,
) {
    publish_serial_event(
        app,
        context,
        "serial.connected",
        "success",
        None,
        "串口会话已连接",
        serde_json::json!({"port": port, "baudRate": baud_rate}),
        false,
    );
}

pub(crate) fn publish_serial_disconnected(
    app: &AppHandle,
    context: &SerialAuditContext,
    port: &str,
    reason: &str,
) {
    publish_serial_event(
        app,
        context,
        "serial.disconnected",
        "success",
        None,
        "串口会话已断开",
        serde_json::json!({"port": port, "reason": reason}),
        true,
    );
}

pub(crate) fn publish_serial_gap(
    app: &AppHandle,
    context: &SerialAuditContext,
    port: &str,
    dropped_bytes: usize,
) {
    publish_serial_event(
        app,
        context,
        "serial.buffer_overflow",
        "failed",
        Some("SERIAL_RECORD_BUFFER_OVERFLOW"),
        "串口记录缓冲发生缺口",
        serde_json::json!({"port": port, "droppedBytes": dropped_bytes}),
        false,
    );
}

fn publish_serial_tx(app: &AppHandle, context: &SerialAuditContext, port: &str, bytes: usize) {
    publish_serial_event(
        app,
        context,
        "serial.tx",
        "success",
        None,
        "串口已发送数据",
        serde_json::json!({"port": port, "byteCount": bytes}),
        false,
    );
}

fn validate_audit_context(context: &SerialAuditContext) -> Result<(), String> {
    let scope_valid = matches!(
        (context.scope_type.as_str(), context.scope_id.as_str()),
        ("linux_board", id) if id.starts_with("linux:")
    ) || matches!(
        (context.scope_type.as_str(), context.scope_id.as_str()),
        ("mcu_board", id) if id.starts_with("mcu:")
    );
    if !scope_valid {
        return Err("INVALID_SERIAL_AUDIT_SCOPE".to_owned());
    }
    let actor_valid = matches!(context.actor_type.as_str(), "user" | "service")
        && !context.actor_id.trim().is_empty();
    let fields = [
        context.actor_id.as_str(),
        context.scope_id.as_str(),
        context.session_id.as_str(),
        context.session_title.as_str(),
    ];
    if !actor_valid
        || fields
            .iter()
            .any(|value| value.len() > 256 || value.chars().any(char::is_control))
        || context.session_id.trim().is_empty()
        || context.session_title.trim().is_empty()
    {
        return Err("INVALID_SERIAL_AUDIT_CONTEXT".to_owned());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn publish_serial_event(
    app: &AppHandle,
    context: &SerialAuditContext,
    action: &str,
    result: &str,
    error_code: Option<&str>,
    summary: &str,
    metadata: serde_json::Value,
    end_session: bool,
) {
    let category = if action == "serial.buffer_overflow" {
        "record_gap"
    } else {
        "serial"
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: context.actor_type.clone(),
            actor_id: context.actor_id.clone(),
            category: category.to_owned(),
            action: action.to_owned(),
            scope_type: context.scope_type.clone(),
            scope_id: context.scope_id.clone(),
            session_id: Some(context.session_id.clone()),
            session_kind: Some("serial".to_owned()),
            session_title: Some(context.session_title.clone()),
            end_session,
            correlation_id: Some(context.session_id.clone()),
            causation_id: None,
            result: result.to_owned(),
            error_code: error_code.map(str::to_owned),
            summary: summary.to_owned(),
            metadata,
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

/// Drain up to `max` bytes from the REST read buffer of an open port (HTTP-only).
/// 前端串口工具走事件路径，不使用此函数。
pub fn serial_read_core(state: &SerialState, port: &str, max: usize) -> Result<Vec<u8>, String> {
    let ports = state
        .ports
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let handle = ports
        .get(port)
        .ok_or_else(|| format!("Port {} is not open", port))?;
    let mut rb = handle
        .read_buffer
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let take = rb.len().min(max);
    Ok(rb.drain(0..take).collect())
}

/// Return the authoritative capture position for the current open session.
/// Unlike the REST/MCP compatibility buffer, querying this status does not
/// consume bytes.
#[command]
pub fn serial_capture_status(
    port: String,
    state: State<'_, Arc<SerialState>>,
) -> Result<SerialCaptureStatus, String> {
    let ports = state.ports.lock().map_err(|e| format!("Lock error: {e}"))?;
    let handle = ports
        .get(&port)
        .ok_or_else(|| format!("Port {port} is not open"))?;
    handle
        .capture
        .status(&port, !handle.disconnected.load(Ordering::Relaxed))
}

/// Read a non-consuming range from the current serial session capture.
#[command]
pub fn serial_capture_read(
    port: String,
    session_id: String,
    offset: u64,
    max_bytes: usize,
    state: State<'_, Arc<SerialState>>,
) -> Result<SerialCaptureChunk, String> {
    let capture = {
        let ports = state.ports.lock().map_err(|e| format!("Lock error: {e}"))?;
        let handle = ports
            .get(&port)
            .ok_or_else(|| format!("Port {port} is not open"))?;
        handle.capture.clone()
    };
    if capture.session_id != session_id {
        return Err(format!(
            "SERIAL_CAPTURE_SESSION:requested={session_id}:active={}",
            capture.session_id
        ));
    }
    capture.read_range(offset, max_bytes)
}

/// Export the current raw capture to a path explicitly chosen by the user.
#[command]
pub fn serial_capture_export(
    port: String,
    session_id: String,
    target: String,
    state: State<'_, Arc<SerialState>>,
) -> Result<u64, String> {
    let capture = {
        let ports = state.ports.lock().map_err(|e| format!("Lock error: {e}"))?;
        let handle = ports
            .get(&port)
            .ok_or_else(|| format!("Port {port} is not open"))?;
        handle.capture.clone()
    };
    if capture.session_id != session_id {
        return Err(format!(
            "SERIAL_CAPTURE_SESSION:requested={session_id}:active={}",
            capture.session_id
        ));
    }
    capture.export_to(std::path::Path::new(&target))
}

/// Check if a serial port is currently open.
#[command]
pub fn serial_is_open(port: String, state: State<'_, Arc<SerialState>>) -> bool {
    serial_is_open_core(&state, &port)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn serial_is_open_core(state: &SerialState, port: &str) -> bool {
    let ports = match state.ports.lock() {
        Ok(p) => p,
        Err(_) => return false,
    };
    ports.get(port).is_some_and(|handle| {
        !handle
            .disconnected
            .load(std::sync::atomic::Ordering::Relaxed)
    })
}

/// Whether LYNX still owns an OS handle for this path, including a handle whose
/// read thread has already observed disconnect. Upgrade/re-enumeration flows
/// must remove that stale entry so Windows can release the COM device stack.
pub fn serial_is_registered_core(state: &SerialState, port: &str) -> bool {
    state
        .ports
        .lock()
        .is_ok_and(|ports| ports.contains_key(port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, vid: u16, pid: u16, serial_number: &str) -> SerialPortInfo {
        SerialPortInfo {
            name: name.to_owned(),
            vid,
            pid,
            manufacturer: String::new(),
            description: String::new(),
            serial_number: serial_number.to_owned(),
            interface_number: None,
        }
    }

    fn audit_context(scope_type: &str, scope_id: &str) -> SerialAuditContext {
        SerialAuditContext {
            actor_type: "service".to_owned(),
            actor_id: "mcu-workbench".to_owned(),
            scope_type: scope_type.to_owned(),
            scope_id: scope_id.to_owned(),
            session_id: "serial-session-1".to_owned(),
            session_title: "board · COM7".to_owned(),
        }
    }

    #[test]
    fn serial_audit_context_rejects_cross_domain_or_unbounded_identity() {
        assert!(validate_audit_context(&audit_context("linux_board", "linux:4")).is_ok());
        assert!(validate_audit_context(&audit_context("mcu_board", "mcu:7")).is_ok());
        assert_eq!(
            validate_audit_context(&audit_context("linux_board", "mcu:7")).unwrap_err(),
            "INVALID_SERIAL_AUDIT_SCOPE"
        );
        let mut invalid = audit_context("mcu_board", "mcu:7");
        invalid.session_title = "x\nsecret".to_owned();
        assert_eq!(
            validate_audit_context(&invalid).unwrap_err(),
            "INVALID_SERIAL_AUDIT_CONTEXT"
        );
    }

    #[test]
    fn high_speed_configuration_is_distinct_and_canonical() {
        let standard = serial_configuration(115_200, 8, 1, "none", "none");
        let high_speed = serial_configuration(1_500_000, 8, 1, "NONE", "none");
        assert_ne!(standard, high_speed);
        assert_eq!(high_speed.baud_rate, 1_500_000);
        assert_eq!(high_speed.parity, "none");
    }

    #[test]
    fn serial_number_survives_port_name_change() {
        let identity = SerialIdentity {
            port: "COM3".to_owned(),
            vid: 0x1a86,
            pid: 0x7523,
            serial_number: "board-01".to_owned(),
            interface_number: None,
        };
        let resolved =
            resolve_port_from(&identity, vec![port("COM19", 0x1a86, 0x7523, "board-01")]).unwrap();
        assert_eq!(resolved.name, "COM19");
    }

    #[cfg(unix)]
    #[test]
    fn explicitly_configured_character_device_supports_virtual_serial_ports() {
        let identity = SerialIdentity {
            port: "/dev/null".to_owned(),
            ..Default::default()
        };
        let resolved = resolve_port_from(&identity, vec![]).unwrap();
        assert_eq!(resolved.name, "/dev/null");
        assert_eq!(
            resolved.description,
            "Configured character-device serial port"
        );
    }

    #[test]
    fn interface_number_distinguishes_shared_usb_serial_number() {
        let identity = SerialIdentity {
            vid: 0x1a86,
            pid: 0x55d5,
            serial_number: "0123456789".to_owned(),
            interface_number: Some(2),
            ..Default::default()
        };
        let mut first = port("/dev/ttyACM0", 0x1a86, 0x55d5, "0123456789");
        first.interface_number = Some(0);
        let mut second = port("/dev/ttyACM1", 0x1a86, 0x55d5, "0123456789");
        second.interface_number = Some(2);
        assert_eq!(
            resolve_port_from(&identity, vec![first, second])
                .unwrap()
                .name,
            "/dev/ttyACM1"
        );
    }

    #[test]
    fn ambiguous_vid_pid_is_rejected() {
        let identity = SerialIdentity {
            vid: 0x1a86,
            pid: 0x7523,
            ..Default::default()
        };
        let error = resolve_port_from(
            &identity,
            vec![
                port("/dev/ttyUSB0", 0x1a86, 0x7523, ""),
                port("/dev/ttyUSB1", 0x1a86, 0x7523, ""),
            ],
        )
        .unwrap_err();
        assert!(error.starts_with("SERIAL_AMBIGUOUS:"));
    }

    #[test]
    fn interface_number_distinguishes_vid_pid_without_reported_serial() {
        let identity = SerialIdentity {
            vid: 0x1a86,
            pid: 0x55d5,
            interface_number: Some(2),
            ..Default::default()
        };
        let mut first = port("/dev/ttyACM0", 0x1a86, 0x55d5, "");
        first.interface_number = Some(0);
        let mut second = port("/dev/ttyACM7", 0x1a86, 0x55d5, "");
        second.interface_number = Some(2);
        assert_eq!(
            resolve_port_from(&identity, vec![first, second])
                .unwrap()
                .name,
            "/dev/ttyACM7"
        );
    }

    #[test]
    fn exact_port_remains_usable_for_legacy_ambiguous_binding() {
        let identity = SerialIdentity {
            port: "/dev/ttyACM1".to_owned(),
            vid: 0x1a86,
            pid: 0x55d5,
            ..Default::default()
        };
        assert_eq!(
            resolve_port_from(
                &identity,
                vec![
                    port("/dev/ttyACM0", 0x1a86, 0x55d5, ""),
                    port("/dev/ttyACM1", 0x1a86, 0x55d5, ""),
                ]
            )
            .unwrap()
            .name,
            "/dev/ttyACM1"
        );
    }
}

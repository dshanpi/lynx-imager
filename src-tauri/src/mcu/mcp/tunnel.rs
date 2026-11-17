//! MCU-channel SSH reverse tunnel.
//!
//! This runtime owns only MCU channel lifecycle and never loads an SDK
//! project. SSH is transport; the board id remains the sole business scope.

use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use ssh2::{Channel, ErrorCode};
use tauri::{AppHandle, Manager, State};

use super::ssh_transport::{self, RemoteTarget};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McuMcpTunnelRequest {
    pub board_id: String,
    pub host_address: String,
    pub host_port: u16,
    pub host_username: String,
    pub auth_type: String,
    pub private_key_path: String,
    pub host_fingerprint: String,
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
    pub local_port: u16,
    pub remote_port: u16,
    pub mcp_server_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McuMcpTunnelStatus {
    pub board_id: String,
    pub channel_kind: &'static str,
    pub state: String,
    pub message: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub endpoint: String,
}

struct Runtime {
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<McuMcpTunnelStatus>>,
}

#[derive(Default)]
pub struct McuMcpTunnelState {
    runtimes: Mutex<HashMap<String, Runtime>>,
}

#[tauri::command]
pub fn mcu_mcp_tunnel_start(
    request: McuMcpTunnelRequest,
    app: AppHandle,
    state: State<'_, McuMcpTunnelState>,
) -> Result<McuMcpTunnelStatus, String> {
    validate(&request)?;
    crate::mcu::boards::board_by_id(&app, &request.board_id).map_err(|error| error.to_string())?;
    let gateway = app.state::<super::gateway::McuMcpGatewayState>();
    if !super::gateway::is_running(gateway.inner(), request.local_port, &request.board_id)? {
        return Err("MCU_MCP_GATEWAY_OFFLINE:请先启动同一板卡的 MCU MCP Gateway".to_owned());
    }
    let remote = ssh_transport::configured_target(
        ssh_transport::RemoteConnectionConfig {
            address: request.host_address.clone(),
            port: request.host_port,
            username: request.host_username.clone(),
            auth_type: request.auth_type.clone(),
            private_key_path: request.private_key_path.clone(),
            host_fingerprint: request.host_fingerprint.clone(),
        },
        request.password.clone(),
        request.key_passphrase.clone(),
    )?;
    let mut runtimes = state.runtimes.lock().map_err(|error| error.to_string())?;
    if let Some(previous) = runtimes.remove(&request.board_id) {
        previous.cancel.store(true, Ordering::SeqCst);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(McuMcpTunnelStatus {
        board_id: request.board_id.clone(),
        channel_kind: "mcu",
        state: "connecting".to_owned(),
        message: "正在连接 MCU MCP 隧道".to_owned(),
        local_port: request.local_port,
        remote_port: request.remote_port,
        endpoint: format!("http://127.0.0.1:{}/mcp", request.remote_port),
    }));
    let runtime_key = request.board_id.clone();
    runtimes.insert(
        runtime_key.clone(),
        Runtime {
            cancel: cancel.clone(),
            status: status.clone(),
        },
    );
    thread::Builder::new()
        .name(format!("lynx-mcu-mcp-{}", request.board_id))
        .spawn(move || supervise(request, remote, cancel, status))
        .map_err(|error| format!("MCU_MCP_TUNNEL_START:{error}"))?;
    snapshot(runtimes.get(&runtime_key).expect("inserted runtime"))
}

#[tauri::command]
pub fn mcu_mcp_tunnel_stop(
    board_id: String,
    state: State<'_, McuMcpTunnelState>,
) -> Result<McuMcpTunnelStatus, String> {
    let runtime = state
        .runtimes
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&board_id)
        .ok_or_else(|| "MCU_MCP_TUNNEL_NOT_RUNNING:通道未运行".to_owned())?;
    runtime.cancel.store(true, Ordering::SeqCst);
    let mut status = snapshot(&runtime)?;
    status.state = "stopped".to_owned();
    status.message = "已停止".to_owned();
    Ok(status)
}

#[tauri::command]
pub fn mcu_mcp_tunnel_status(
    board_id: String,
    state: State<'_, McuMcpTunnelState>,
) -> Result<Option<McuMcpTunnelStatus>, String> {
    let runtimes = state.runtimes.lock().map_err(|error| error.to_string())?;
    runtimes.get(&board_id).map(snapshot).transpose()
}

fn validate(request: &McuMcpTunnelRequest) -> Result<(), String> {
    if request.local_port == 0
        || request.remote_port == 0
        || request.mcp_server_name.trim().is_empty()
    {
        return Err("MCU_MCP_VALIDATION:端口和 MCP 名称不能为空".to_owned());
    }
    if !request
        .mcp_server_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(
            "MCU_MCP_VALIDATION:MCP 名称只能包含 ASCII 字母、数字、下划线和连字符".to_owned(),
        );
    }
    Ok(())
}

fn snapshot(runtime: &Runtime) -> Result<McuMcpTunnelStatus, String> {
    runtime
        .status
        .lock()
        .map(|status| status.clone())
        .map_err(|error| error.to_string())
}
fn set_status(status: &Arc<Mutex<McuMcpTunnelStatus>>, state: &str, message: impl Into<String>) {
    if let Ok(mut value) = status.lock() {
        value.state = state.to_owned();
        value.message = message.into();
    }
}

fn supervise(
    request: McuMcpTunnelRequest,
    remote: RemoteTarget,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<McuMcpTunnelStatus>>,
) {
    while !cancel.load(Ordering::SeqCst) {
        set_status(&status, "connecting", "正在连接 MCU MCP 隧道");
        if let Err(error) = forward_once(&request, &remote, &cancel, &status) {
            set_status(&status, "reconnecting", format!("{error}；3 秒后重连"));
        }
        for _ in 0..30 {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    set_status(&status, "stopped", "已停止");
}

fn forward_once(
    request: &McuMcpTunnelRequest,
    remote: &RemoteTarget,
    cancel: &Arc<AtomicBool>,
    status: &Arc<Mutex<McuMcpTunnelStatus>>,
) -> Result<(), String> {
    let session = ssh_transport::connect(remote)?;
    session.set_keepalive(true, 15);
    let (mut listener, bound) = session
        .channel_forward_listen(request.remote_port, Some("127.0.0.1"), Some(32))
        .map_err(|error| format!("MCU_MCP_FORWARD:{error}"))?;
    if bound != request.remote_port {
        return Err(format!(
            "MCU_MCP_FORWARD:请求端口 {}，实际端口 {bound}",
            request.remote_port
        ));
    }
    session.set_blocking(false);
    set_status(
        status,
        "connected",
        format!(
            "MCU 专用通道：http://127.0.0.1:{bound}/mcp → 本机 127.0.0.1:{}",
            request.local_port
        ),
    );
    while !cancel.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok(channel) => {
                let local_port = request.local_port;
                let relay_cancel = cancel.clone();
                thread::spawn(move || relay(channel, local_port, relay_cancel));
            }
            Err(error) if matches!(error.code(), ErrorCode::Session(-37)) => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(format!("MCU_MCP_ACCEPT:{error}")),
        }
        let _ = session.keepalive_send();
    }
    Ok(())
}

fn relay(channel: Channel, local_port: u16, cancel: Arc<AtomicBool>) {
    let Ok(local) = TcpStream::connect(("127.0.0.1", local_port)) else {
        return;
    };
    let _ = local.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = local.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(mut local_writer) = local.try_clone() else {
        return;
    };
    let mut local_reader = local;
    let mut ssh_reader = channel.stream(0);
    let mut ssh_writer = channel.stream(0);
    let done = Arc::new(AtomicBool::new(false));
    let upload_done = done.clone();
    let upload_cancel = cancel.clone();
    let upload = thread::spawn(move || {
        let mut buffer = [0u8; 32768];
        while !upload_done.load(Ordering::SeqCst) && !upload_cancel.load(Ordering::SeqCst) {
            match local_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    if ssh_writer.write_all(&buffer[..size]).is_err() {
                        break;
                    }
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        upload_done.store(true, Ordering::SeqCst);
    });
    let mut buffer = [0u8; 32768];
    while !done.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
        match ssh_reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if local_writer.write_all(&buffer[..size]).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => break,
        }
    }
    done.store(true, Ordering::SeqCst);
    let _ = upload.join();
}

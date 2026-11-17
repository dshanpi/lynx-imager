use std::collections::HashSet;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rand::Rng;
use ssh2::{Channel, ErrorCode};
use tauri::State;

use super::types::{HostConfig, ProjectConfig, SshCredentials, TunnelStatus};
use super::{credentials, ssh, storage, WorkbenchState};

pub struct TunnelRuntime {
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<TunnelStatus>>,
}

impl TunnelRuntime {
    fn snapshot(&self) -> Result<TunnelStatus, String> {
        self.status
            .lock()
            .map(|value| value.clone())
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn workbench_start_tunnel(
    project_id: i64,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<TunnelStatus, String> {
    start_tunnel_inner(project_id, state.inner(), true)
}

/// Start and supervise a saved tunnel without requiring a frontend command.
///
/// This is also used during application startup so tunnels recover regardless
/// of which page the user opens first.
pub fn start_tunnel(project_id: i64, state: &Arc<WorkbenchState>) -> Result<TunnelStatus, String> {
    start_tunnel_inner(project_id, state, false)
}

/// Wait for the current project tunnel to become usable and return the actual
/// server-allocated remote port. Agent terminals must not assume that the
/// preferred port survived remote binding.
pub fn wait_for_tunnel(
    project_id: i64,
    state: &Arc<WorkbenchState>,
    timeout: Duration,
) -> Result<TunnelStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = {
            let tunnels = state.tunnels.lock().map_err(|e| e.to_string())?;
            tunnels
                .get(&project_id)
                .ok_or_else(|| "TUNNEL_NOT_RUNNING:通道未运行".to_owned())?
                .snapshot()?
        };
        if snapshot.state == "connected" {
            return Ok(snapshot);
        }
        if snapshot.state == "host_key_required" {
            return Err(snapshot.message);
        }
        if snapshot.state == "stopped" {
            return Err("TUNNEL_STOPPED:MCP 通道已停止".to_owned());
        }
        if Instant::now() >= deadline {
            return Err(format!("TUNNEL_READY_TIMEOUT:state={}", snapshot.state));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn start_tunnel_inner(
    project_id: i64,
    state: &Arc<WorkbenchState>,
    reload_if_running: bool,
) -> Result<TunnelStatus, String> {
    let (project, host) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let project = storage::list_projects(&db)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|item| item.id == Some(project_id))
            .ok_or_else(|| "TUNNEL_PROJECT:SDK 通道不存在".to_owned())?;
        let host = storage::list_hosts(&db)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|item| item.id == Some(project.host_id))
            .ok_or_else(|| "TUNNEL_HOST:远端主机不存在".to_owned())?;
        (project, host)
    };
    let credentials = credentials::resolve(host.id, SshCredentials::default())?;

    let mut tunnels = state.tunnels.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = tunnels.get(&project_id) {
        let snapshot = existing.snapshot()?;
        if !reload_if_running
            && matches!(
                snapshot.state.as_str(),
                "connecting" | "verifying" | "connected" | "host_key_required"
            )
        {
            return Ok(snapshot);
        }
    }
    if let Some(existing) = tunnels.remove(&project_id) {
        existing.cancel.store(true, Ordering::SeqCst);
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(TunnelStatus {
        project_id,
        state: "connecting".to_owned(),
        message: "正在连接远端 SDK 主机".to_owned(),
        local_port: project.local_gateway_port,
        remote_port: project.remote_forward_port,
    }));
    let runtime = TunnelRuntime {
        cancel: cancel.clone(),
        status: status.clone(),
    };
    tunnels.insert(project_id, runtime);
    let workbench = state.clone();
    thread::Builder::new()
        .name(format!("lynx-tunnel-{project_id}"))
        .spawn(move || supervise(project, host, credentials, cancel, status, workbench))
        .map_err(|e| format!("TUNNEL_START:{e}"))?;
    tunnels
        .get(&project_id)
        .ok_or_else(|| "TUNNEL_START:运行状态丢失".to_owned())?
        .snapshot()
}

#[tauri::command]
pub fn workbench_stop_tunnel(
    project_id: i64,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<TunnelStatus, String> {
    let mut tunnels = state.tunnels.lock().map_err(|e| e.to_string())?;
    let runtime = tunnels
        .remove(&project_id)
        .ok_or_else(|| "TUNNEL_NOT_RUNNING:通道未运行".to_owned())?;
    runtime.cancel.store(true, Ordering::SeqCst);
    let mut snapshot = runtime.snapshot()?;
    snapshot.state = "stopped".to_owned();
    snapshot.message = "已停止".to_owned();
    Ok(snapshot)
}

#[tauri::command]
pub fn workbench_list_tunnel_statuses(
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<TunnelStatus>, String> {
    let tunnels = state.tunnels.lock().map_err(|e| e.to_string())?;
    tunnels.values().map(TunnelRuntime::snapshot).collect()
}

fn supervise(
    mut project: ProjectConfig,
    host: HostConfig,
    credentials: SshCredentials,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<TunnelStatus>>,
    workbench: Arc<WorkbenchState>,
) {
    while !cancel.load(Ordering::SeqCst) {
        set_status(&status, "connecting", "正在连接远端 SDK 主机");
        match run_once(
            &mut project,
            &host,
            &credentials,
            &cancel,
            &status,
            &workbench,
        ) {
            Ok(()) if cancel.load(Ordering::SeqCst) => break,
            Ok(()) => set_status(&status, "reconnecting", "SSH 连接已断开，3 秒后自动重连"),
            Err(error) if is_host_key_error(&error) => {
                set_status(
                    &status,
                    "host_key_required",
                    &format!("{error}；请在单板设置的 SDK 主机配置中核验指纹，再保存通道连接"),
                );
                return;
            }
            Err(error) => set_status(&status, "reconnecting", &format!("{error}；3 秒后自动重连")),
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

fn is_host_key_error(error: &str) -> bool {
    error.starts_with("HOST_KEY_CHANGED:") || error.starts_with("HOST_KEY_CONFIRM_REQUIRED:")
}

fn run_once(
    project: &mut ProjectConfig,
    host: &HostConfig,
    credentials: &SshCredentials,
    cancel: &Arc<AtomicBool>,
    status: &Arc<Mutex<TunnelStatus>>,
    workbench: &Arc<WorkbenchState>,
) -> Result<(), String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_tunnel");
    let (session, _, _) = ssh::connect(host, credentials)?;
    session.set_keepalive(true, 15);
    let requested_port = project.remote_forward_port;
    let (mut listener, bound_port) = bind_remote_port(requested_port, |port| {
        session.channel_forward_listen(port, Some("127.0.0.1"), Some(32))
    })?;
    if bound_port == 0 {
        return Err("TUNNEL_FORWARD:SSH 服务端返回了无效端口 0".to_owned());
    }
    if bound_port != requested_port {
        project.remote_forward_port = bound_port;
        let db = workbench.db.lock().map_err(|error| error.to_string())?;
        storage::save_project(&db, project).map_err(|error| format!("TUNNEL_PORT_SAVE:{error}"))?;
        log::info!(
            "Remote MCP port {requested_port} was occupied; SSH allocated and persisted {bound_port}"
        );
    }
    set_remote_port(status, bound_port);
    session.set_blocking(false);
    set_status(
        status,
        "verifying",
        &format!("SSH 已请求映射，正在从远端验证 127.0.0.1:{bound_port}"),
    );
    let mut probe = Some(spawn_remote_probe(
        host,
        credentials,
        bound_port,
        cancel,
        Duration::from_millis(250),
    ));

    let mut last_keepalive = Instant::now();
    let mut probe_deadline = Instant::now() + Duration::from_secs(8);
    let mut next_probe_at = Instant::now() + Duration::from_secs(10);
    let mut verified = false;
    while !cancel.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok(channel) => {
                let local_port = project.local_gateway_port;
                let connection_cancel = cancel.clone();
                thread::spawn(move || relay(channel, local_port, connection_cancel));
            }
            Err(error) if is_would_block(&error) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("TUNNEL_ACCEPT:{error}")),
        }
        if let Some(probe_receiver) = &probe {
            match probe_receiver.try_recv() {
                Ok(Ok(())) => {
                    if !verified {
                        verified = true;
                        set_status(
                            status,
                            "connected",
                            &format!(
                                "远端实测通过：http://127.0.0.1:{bound_port}/mcp → 本机 127.0.0.1:{}",
                                project.local_gateway_port
                            ),
                        );
                    } else {
                        log::debug!(
                            "SSH tunnel health check passed: remote 127.0.0.1:{bound_port} -> local 127.0.0.1:{}",
                            project.local_gateway_port
                        );
                    }
                    probe = None;
                    next_probe_at = Instant::now() + Duration::from_secs(10);
                }
                Ok(Err(error)) => {
                    return Err(format!("TUNNEL_REMOTE_HEALTH:{error}"));
                }
                Err(mpsc::TryRecvError::Empty) if Instant::now() >= probe_deadline => {
                    return Err("TUNNEL_REMOTE_PROBE:远端端口验证超时".to_owned());
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("TUNNEL_REMOTE_PROBE:验证线程异常退出".to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if verified && probe.is_none() && Instant::now() >= next_probe_at {
            probe = Some(spawn_remote_probe(
                host,
                credentials,
                bound_port,
                cancel,
                Duration::ZERO,
            ));
            probe_deadline = Instant::now() + Duration::from_secs(8);
        }
        if last_keepalive.elapsed() >= Duration::from_secs(10) {
            match session.keepalive_send() {
                Ok(_) => last_keepalive = Instant::now(),
                Err(error) if is_would_block(&error) => {}
                Err(error) => return Err(format!("TUNNEL_KEEPALIVE:{error}")),
            }
        }
    }
    Ok(())
}

fn spawn_remote_probe(
    host: &HostConfig,
    credentials: &SshCredentials,
    remote_port: u16,
    cancel: &Arc<AtomicBool>,
    delay: Duration,
) -> mpsc::Receiver<Result<(), String>> {
    let (sender, receiver) = mpsc::channel();
    let probe_host = host.clone();
    let probe_credentials = credentials.clone();
    let probe_cancel = cancel.clone();
    thread::spawn(move || {
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        if probe_cancel.load(Ordering::SeqCst) {
            return;
        }
        let _ = sender.send(probe_remote_endpoint(
            &probe_host,
            &probe_credentials,
            remote_port,
        ));
    });
    receiver
}

fn probe_remote_endpoint(
    host: &HostConfig,
    credentials: &SshCredentials,
    remote_port: u16,
) -> Result<(), String> {
    let (session, _, _) = ssh::connect(host, credentials)?;
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("创建远端验证会话失败：{e}"))?;
    channel
        .exec(&format!(
            "curl -fsS --max-time 4 http://127.0.0.1:{remote_port}/healthz"
        ))
        .map_err(|e| format!("执行远端验证失败：{e}"))?;
    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|e| format!("读取远端验证结果失败：{e}"))?;
    channel
        .wait_close()
        .map_err(|e| format!("等待远端验证结束失败：{e}"))?;
    let exit_status = channel
        .exit_status()
        .map_err(|e| format!("读取远端验证状态失败：{e}"))?;
    if exit_status != 0 || !output.contains("\"status\":\"ok\"") {
        return Err(format!(
            "远端 http://127.0.0.1:{remote_port}/healthz 不可用（exit={exit_status}）"
        ));
    }
    Ok(())
}

fn relay(channel: Channel, local_port: u16, tunnel_cancel: Arc<AtomicBool>) {
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
    let upload_cancel = tunnel_cancel.clone();

    let upload = thread::spawn(move || {
        let mut buffer = [0u8; 32 * 1024];
        while !upload_done.load(Ordering::SeqCst) && !upload_cancel.load(Ordering::SeqCst) {
            match local_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size)
                    if write_retry(&mut ssh_writer, &buffer[..size], &upload_done).is_err() =>
                {
                    break;
                }
                Ok(_) => {}
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        upload_done.store(true, Ordering::SeqCst);
    });

    let mut buffer = [0u8; 32 * 1024];
    while !done.load(Ordering::SeqCst) && !tunnel_cancel.load(Ordering::SeqCst) {
        match ssh_reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) if local_writer.write_all(&buffer[..size]).is_err() => break,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
    done.store(true, Ordering::SeqCst);
    let _ = upload.join();
}

fn write_retry(
    writer: &mut impl Write,
    mut bytes: &[u8],
    done: &AtomicBool,
) -> std::io::Result<()> {
    while !bytes.is_empty() && !done.load(Ordering::SeqCst) {
        match writer.write(bytes) {
            Ok(0) => return Err(std::io::Error::from(ErrorKind::WriteZero)),
            Ok(size) => bytes = &bytes[size..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn is_would_block(error: &ssh2::Error) -> bool {
    matches!(error.code(), ErrorCode::Session(-37))
}

fn bind_remote_port<T, E>(
    requested_port: u16,
    mut listen: impl FnMut(u16) -> Result<(T, u16), E>,
) -> Result<(T, u16), String>
where
    E: std::fmt::Display,
{
    let requested_error = match listen(requested_port) {
        Ok(value) => return Ok(value),
        Err(error) => error.to_string(),
    };

    // OpenSSH supports port 0 and returns the port selected by the server.
    // Some SSH servers reject that extension even though ordinary remote
    // forwarding is enabled, so fall back to atomic listen requests for
    // random high ports. The listen request itself performs the bind; there
    // is no availability-check/bind race between concurrent LYNX clients.
    let allocation_error = match listen(0) {
        Ok(value) => return Ok(value),
        Err(error) => error.to_string(),
    };

    let mut rng = rand::thread_rng();
    let mut candidates = HashSet::new();
    let mut last_fallback_error = None;
    while candidates.len() < 32 {
        let candidate = rng.gen_range(20_000..=60_000);
        if candidate == requested_port || !candidates.insert(candidate) {
            continue;
        }
        match listen(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => last_fallback_error = Some(error.to_string()),
        }
    }

    Err(format!(
        "TUNNEL_FORWARD:端口 {requested_port} 不可用（{requested_error}），服务端端口 0 分配失败（{allocation_error}），兼容避让 32 次也失败（{}）",
        last_fallback_error.unwrap_or_else(|| "SSH 服务端拒绝远端转发".to_owned())
    ))
}

fn set_status(status: &Arc<Mutex<TunnelStatus>>, state: &str, message: &str) {
    log::info!("SSH tunnel state={state}: {message}");
    if let Ok(mut value) = status.lock() {
        value.state = state.to_owned();
        value.message = message.to_owned();
    }
}

fn set_remote_port(status: &Arc<Mutex<TunnelStatus>>, remote_port: u16) {
    if let Ok(mut value) = status.lock() {
        value.remote_port = remote_port;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_identity_errors_require_operator_action_instead_of_reconnection() {
        assert!(is_host_key_error(
            "HOST_KEY_CHANGED:expected=old,actual=new"
        ));
        assert!(is_host_key_error("HOST_KEY_CONFIRM_REQUIRED:SHA256:key"));
        assert!(!is_host_key_error("SSH_CONNECT:timeout"));
        assert!(!is_host_key_error("SSH_AUTH:denied"));
    }

    #[test]
    fn tunnel_status_tracks_server_allocated_remote_port() {
        let status = Arc::new(Mutex::new(TunnelStatus {
            project_id: 1,
            state: "connecting".to_owned(),
            message: String::new(),
            local_port: 8765,
            remote_port: 18765,
        }));
        set_remote_port(&status, 42317);
        assert_eq!(status.lock().unwrap().remote_port, 42317);
    }

    #[test]
    fn occupied_remote_port_falls_back_to_atomic_server_allocation() {
        let mut requested = Vec::new();
        let (listener, bound_port) = bind_remote_port(18765, |port| {
            requested.push(port);
            if port == 18765 {
                Err("address already in use")
            } else {
                Ok(("listener", 42317))
            }
        })
        .unwrap();
        assert_eq!(requested, [18765, 0]);
        assert_eq!(listener, "listener");
        assert_eq!(bound_port, 42317);
    }

    #[test]
    fn server_without_port_zero_support_falls_back_to_atomic_random_bind() {
        let mut requested = Vec::new();
        let (listener, bound_port) = bind_remote_port(18765, |port| {
            requested.push(port);
            match port {
                18765 => Err("address already in use"),
                0 => Err("port zero unsupported"),
                candidate => Ok(("listener", candidate)),
            }
        })
        .unwrap();
        assert_eq!(requested[0..2], [18765, 0]);
        assert!((20_000..=60_000).contains(&requested[2]));
        assert_eq!(listener, "listener");
        assert_eq!(bound_port, requested[2]);
    }
}

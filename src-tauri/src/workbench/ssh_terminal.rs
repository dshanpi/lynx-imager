//! Interactive, project-scoped SSH PTY sessions.
//!
//! A dedicated worker thread owns each libssh2 session. The UI only receives
//! bounded terminal bytes and cannot silently reconnect or replay input.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Emitter, State};

use super::commands::{begin_remote_task_audit, finish_remote_task_audit, RemoteTaskActor};
use super::types::{AiProvider, HostConfig, ProjectConfig, SshCredentials};
use super::{credentials, ssh, storage, WorkbenchState};

const MAX_TERMINALS: usize = 8;
const MAX_RETAINED_BYTES: usize = 1024 * 1024;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const ACTION_QUEUE_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshTerminalMode {
    Shell,
    Codex,
    ClaudeCode,
    DeepseekHarness,
}

impl SshTerminalMode {
    fn label(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::DeepseekHarness => "deepseek-harness",
        }
    }

    fn needs_mcp(self) -> bool {
        !matches!(self, Self::Shell)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalSnapshot {
    pub terminal_id: String,
    pub project_id: i64,
    pub device_id: i64,
    pub mode: SshTerminalMode,
    pub title: String,
    pub working_directory: String,
    pub state: String,
    pub message: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub output_epoch: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SshTerminalDataEvent {
    terminal_id: String,
    project_id: i64,
    device_id: i64,
    output_epoch: u64,
    received_bytes: u64,
    data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalClearResult {
    output_epoch: u64,
    received_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SshTerminalStatusEvent {
    terminal_id: String,
    project_id: i64,
    device_id: i64,
    state: String,
    message: String,
    ended_at_ms: Option<u64>,
    exit_code: Option<i32>,
    sent_bytes: u64,
    received_bytes: u64,
}

enum TerminalAction {
    Write(Vec<u8>),
    Resize(u32, u32),
    Close,
}

pub struct SshTerminalRuntime {
    action_tx: SyncSender<TerminalAction>,
    snapshot: Arc<Mutex<SshTerminalSnapshot>>,
    input_owner: Mutex<Option<String>>,
}

impl SshTerminalRuntime {
    fn snapshot(&self) -> Result<SshTerminalSnapshot, String> {
        self.snapshot
            .lock()
            .map(|value| value.clone())
            .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE".to_owned())
    }
}

#[command]
pub fn workbench_ssh_terminal_open(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    project_id: i64,
    device_id: i64,
    mode: SshTerminalMode,
    cols: u32,
    rows: u32,
) -> Result<SshTerminalSnapshot, String> {
    validate_size(cols, rows)?;
    let (mut project, host) = load_scope(&state, project_id, device_id)?;
    validate_project(&project)?;
    let credentials = credentials::resolve(host.id, SshCredentials::default())?;

    let mut terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let active_count = terminals
        .values()
        .filter_map(|runtime| runtime.snapshot().ok())
        .filter(|snapshot| matches!(snapshot.state.as_str(), "connecting" | "connected"))
        .count();
    if active_count >= MAX_TERMINALS {
        return Err(format!(
            "SSH_TERMINAL_LIMIT:maximum {MAX_TERMINALS} sessions"
        ));
    }

    let terminal_id = format!("ssh-{}-{:016x}", now_ms(), rand::random::<u64>());
    let audit = begin_remote_task_audit(
        &app,
        &terminal_id,
        "terminal",
        RemoteTaskActor::User,
        &host,
        Some(&project),
        serde_json::json!({
            "projectId": project_id,
            "deviceId": device_id,
            "mode": mode.label(),
        }),
    )?;

    if mode.needs_mcp() {
        if let Err(error) = super::tunnel::start_tunnel(project_id, state.inner()) {
            let result: Result<(), String> = Err(error.clone());
            finish_remote_task_audit(&app, &audit, "failed", &result);
            return Err(format!("SSH_TERMINAL_MCP:{error}"));
        }
        match super::tunnel::wait_for_tunnel(project_id, state.inner(), Duration::from_secs(12)) {
            Ok(status) => project.remote_forward_port = status.remote_port,
            Err(error) => {
                let result: Result<(), String> = Err(error.clone());
                finish_remote_task_audit(&app, &audit, "failed", &result);
                return Err(format!("SSH_TERMINAL_MCP:{error}"));
            }
        }
    }

    let started_at_ms = now_ms();
    let snapshot = Arc::new(Mutex::new(SshTerminalSnapshot {
        terminal_id: terminal_id.clone(),
        project_id,
        device_id,
        mode,
        title: next_title(&terminals, project_id, mode),
        working_directory: project.sdk_dir.trim().to_owned(),
        state: "connecting".to_owned(),
        message: "正在建立 SSH 终端会话".to_owned(),
        started_at_ms,
        ended_at_ms: None,
        exit_code: None,
        received_bytes: 0,
        sent_bytes: 0,
        output_epoch: 0,
        data: Vec::new(),
    }));
    let initial = snapshot
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?
        .clone();
    let (action_tx, action_rx) = mpsc::sync_channel(ACTION_QUEUE_DEPTH);
    terminals.insert(
        terminal_id.clone(),
        SshTerminalRuntime {
            action_tx,
            snapshot: snapshot.clone(),
            input_owner: Mutex::new(None),
        },
    );
    drop(terminals);

    thread::Builder::new()
        .name(format!("lynx-{terminal_id}"))
        .spawn(move || {
            let result = run_terminal(
                &app,
                &terminal_id,
                project_id,
                device_id,
                &host,
                &project,
                credentials,
                mode,
                cols,
                rows,
                action_rx,
                &snapshot,
            );
            if let Err(error) = &result {
                if let Ok(mut value) = snapshot.lock() {
                    value.ended_at_ms = Some(now_ms());
                }
                update_status(&app, &snapshot, "failed", &safe_status_message(error));
            }
            let status = if result.is_ok() { "success" } else { "failed" };
            finish_remote_task_audit(&app, &audit, status, &result);
        })
        .map_err(|error| format!("SSH_TERMINAL_THREAD:{error}"))?;

    Ok(initial)
}

#[command]
pub fn workbench_ssh_terminal_list(
    state: State<'_, Arc<WorkbenchState>>,
    project_id: i64,
    device_id: i64,
) -> Result<Vec<SshTerminalSnapshot>, String> {
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let mut result = terminals
        .values()
        .filter_map(|runtime| runtime.snapshot().ok())
        .filter(|item| item.project_id == project_id && item.device_id == device_id)
        .collect::<Vec<_>>();
    result.sort_by_key(|item| item.started_at_ms);
    Ok(result)
}

#[command]
pub fn workbench_ssh_terminal_snapshot(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
) -> Result<SshTerminalSnapshot, String> {
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let snapshot = terminals
        .get(&terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?
        .snapshot()?;
    ensure_snapshot_scope(&snapshot, project_id, device_id)?;
    Ok(snapshot)
}

#[command]
pub fn workbench_ssh_terminal_claim_input(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
    surface_id: String,
) -> Result<(), String> {
    validate_input_surface_id(&surface_id)?;
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let runtime = terminals
        .get(&terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?;
    ensure_snapshot_scope(&runtime.snapshot()?, project_id, device_id)?;
    *runtime
        .input_owner
        .lock()
        .map_err(|_| "SSH_TERMINAL_INPUT_OWNER_UNAVAILABLE")? = Some(surface_id);
    Ok(())
}

#[command]
pub fn workbench_ssh_terminal_write(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
    surface_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    if data.is_empty() {
        return Ok(());
    }
    if data.len() > MAX_WRITE_BYTES {
        return Err("SSH_TERMINAL_WRITE_TOO_LARGE".to_owned());
    }
    validate_input_surface_id(&surface_id)?;
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let runtime = terminals
        .get(&terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?;
    ensure_snapshot_scope(&runtime.snapshot()?, project_id, device_id)?;
    let owner = runtime
        .input_owner
        .lock()
        .map_err(|_| "SSH_TERMINAL_INPUT_OWNER_UNAVAILABLE")?;
    ensure_input_surface_owner(owner.as_deref(), &surface_id)?;
    runtime
        .action_tx
        .try_send(TerminalAction::Write(data))
        .map_err(|_| "SSH_TERMINAL_BACKPRESSURE".to_owned())
}

fn validate_input_surface_id(surface_id: &str) -> Result<(), String> {
    if surface_id.is_empty()
        || surface_id.len() > 128
        || !surface_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
    {
        return Err("SSH_TERMINAL_INPUT_SURFACE_INVALID".to_owned());
    }
    Ok(())
}

fn ensure_input_surface_owner(owner: Option<&str>, surface_id: &str) -> Result<(), String> {
    if owner != Some(surface_id) {
        return Err("SSH_TERMINAL_INPUT_SURFACE_STALE".to_owned());
    }
    Ok(())
}

#[command]
pub fn workbench_ssh_terminal_resize(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    validate_size(cols, rows)?;
    send_action(
        &state,
        &terminal_id,
        project_id,
        device_id,
        TerminalAction::Resize(cols, rows),
    )
}

#[command]
pub fn workbench_ssh_terminal_clear(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
) -> Result<SshTerminalClearResult, String> {
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let runtime = terminals
        .get(&terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?;
    let mut snapshot = runtime
        .snapshot
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    ensure_snapshot_scope(&snapshot, project_id, device_id)?;
    Ok(clear_retained_output(&mut snapshot))
}

fn clear_retained_output(snapshot: &mut SshTerminalSnapshot) -> SshTerminalClearResult {
    snapshot.data.clear();
    snapshot.output_epoch = snapshot.output_epoch.saturating_add(1);
    SshTerminalClearResult {
        output_epoch: snapshot.output_epoch,
        received_bytes: snapshot.received_bytes,
    }
}

#[command]
pub fn workbench_ssh_terminal_close(
    state: State<'_, Arc<WorkbenchState>>,
    terminal_id: String,
    project_id: i64,
    device_id: i64,
) -> Result<(), String> {
    let mut terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let runtime = terminals
        .get(&terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?;
    ensure_snapshot_scope(&runtime.snapshot()?, project_id, device_id)?;
    match runtime.action_tx.try_send(TerminalAction::Close) {
        Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => {
            terminals.remove(&terminal_id);
        }
        Err(mpsc::TrySendError::Full(_)) => {
            return Err("SSH_TERMINAL_BACKPRESSURE".to_owned());
        }
    }
    Ok(())
}

pub fn shutdown_all(state: &WorkbenchState) {
    if let Ok(mut terminals) = state.ssh_terminals.lock() {
        for (_, runtime) in terminals.drain() {
            let _ = runtime.action_tx.try_send(TerminalAction::Close);
        }
    }
}

fn send_action(
    state: &WorkbenchState,
    terminal_id: &str,
    project_id: i64,
    device_id: i64,
    action: TerminalAction,
) -> Result<(), String> {
    let terminals = state
        .ssh_terminals
        .lock()
        .map_err(|_| "SSH_TERMINAL_STATE_UNAVAILABLE")?;
    let runtime = terminals
        .get(terminal_id)
        .ok_or_else(|| "SSH_TERMINAL_NOT_FOUND".to_owned())?;
    ensure_snapshot_scope(&runtime.snapshot()?, project_id, device_id)?;
    runtime
        .action_tx
        .try_send(action)
        .map_err(|_| "SSH_TERMINAL_BACKPRESSURE".to_owned())
}

#[allow(clippy::too_many_arguments)]
fn run_terminal(
    app: &AppHandle,
    terminal_id: &str,
    project_id: i64,
    device_id: i64,
    host: &HostConfig,
    project: &ProjectConfig,
    credentials: SshCredentials,
    mode: SshTerminalMode,
    cols: u32,
    rows: u32,
    actions: Receiver<TerminalAction>,
    snapshot: &Arc<Mutex<SshTerminalSnapshot>>,
) -> Result<(), String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_terminal");
    let (session, _, _) = if mode.needs_mcp() {
        ssh::connect(host, &credentials)?
    } else {
        ssh::connect_interactive_terminal(host, &credentials)?
    };
    let mut channel = session
        .channel_session()
        .map_err(|error| format!("SSH_TERMINAL_CHANNEL:{error}"))?;
    channel
        .request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))
        .map_err(|error| format!("SSH_TERMINAL_PTY:{error}"))?;
    channel
        .exec(&terminal_command(host, project, mode)?)
        .map_err(|error| format!("SSH_TERMINAL_EXEC:{error}"))?;
    session.set_blocking(false);
    update_status(app, snapshot, "connected", "SSH PTY 已连接");

    let mut pending = VecDeque::<u8>::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut close_requested = false;
    let mut last_status = Instant::now();
    while !close_requested {
        loop {
            match actions.try_recv() {
                Ok(TerminalAction::Write(bytes)) => {
                    if pending.len() + bytes.len() > MAX_WRITE_BYTES * 2 {
                        return Err("SSH_TERMINAL_INPUT_OVERFLOW".to_owned());
                    }
                    pending.extend(bytes);
                }
                Ok(TerminalAction::Resize(cols, rows)) => {
                    if let Err(error) = channel.request_pty_size(cols, rows, None, None) {
                        if error.code() != ssh2::ErrorCode::Session(-37) {
                            return Err(format!("SSH_TERMINAL_RESIZE:{error}"));
                        }
                    }
                }
                Ok(TerminalAction::Close) | Err(TryRecvError::Disconnected) => {
                    close_requested = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        if !pending.is_empty() {
            let contiguous = pending.make_contiguous();
            match channel.write(contiguous) {
                Ok(count) => {
                    pending.drain(..count);
                    increment_sent(snapshot, count as u64);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(format!("SSH_TERMINAL_WRITE:{error}")),
            }
        }

        let mut received = false;
        loop {
            match channel.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    received = true;
                    append_output(
                        app,
                        snapshot,
                        terminal_id,
                        project_id,
                        device_id,
                        &buffer[..count],
                    );
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("SSH_TERMINAL_READ:{error}")),
            }
        }
        if channel.eof() {
            break;
        }
        if last_status.elapsed() >= Duration::from_secs(5) {
            emit_status(app, snapshot);
            last_status = Instant::now();
        }
        if !received {
            thread::sleep(Duration::from_millis(12));
        }
    }

    let _ = channel.close();
    let _ = channel.wait_close();
    let exit_code = channel.exit_status().ok();
    if let Ok(mut value) = snapshot.lock() {
        value.ended_at_ms = Some(now_ms());
        value.exit_code = exit_code;
    }
    update_status(
        app,
        snapshot,
        "closed",
        if close_requested {
            "会话已关闭"
        } else {
            "远端会话已结束"
        },
    );
    Ok(())
}

fn terminal_command(
    host: &HostConfig,
    project: &ProjectConfig,
    mode: SshTerminalMode,
) -> Result<String, String> {
    let sdk = ssh::shell_quote(project.sdk_dir.trim());
    let command = match mode {
        SshTerminalMode::Shell => "exec bash -li".to_owned(),
        SshTerminalMode::Codex => {
            let executable = interactive_executable(host, AiProvider::Codex)?;
            let executable = ssh::shell_quote(&executable);
            let mcp_name = validated_mcp_name(project)?;
            let config = format!(
                "mcp_servers.{mcp_name}.url=\"http://127.0.0.1:{}/mcp\"",
                project.remote_forward_port
            );
            format!(
                "if ! command -v {executable} >/dev/null 2>&1; then printf '\\033[31m[LYNX] Codex executable not found: %s\\033[0m\\r\\n' {executable}; exit 127; fi; exec {executable} -c {}",
                ssh::shell_quote(&config)
            )
        }
        SshTerminalMode::ClaudeCode => {
            let executable = interactive_executable(host, AiProvider::ClaudeCode)?;
            let executable = ssh::shell_quote(&executable);
            let mcp_name = validated_mcp_name(project)?;
            let json = serde_json::json!({
                "mcpServers": {
                    (mcp_name): {
                        "type": "http",
                        "url": format!("http://127.0.0.1:{}/mcp", project.remote_forward_port)
                    }
                }
            })
            .to_string();
            format!(
                "if ! command -v {executable} >/dev/null 2>&1; then printf '\\033[31m[LYNX] Claude executable not found: %s\\033[0m\\r\\n' {executable}; exit 127; fi; mcp_tmp=$(mktemp) || exit 1; chmod 600 \"$mcp_tmp\"; printf '%s' {} >\"$mcp_tmp\"; trap 'rm -f \"$mcp_tmp\"' EXIT HUP INT TERM; {executable} --mcp-config \"$mcp_tmp\"",
                ssh::shell_quote(&json),
            )
        }
        SshTerminalMode::DeepseekHarness => {
            let mcp_name = validated_mcp_name(project)?;
            if mcp_name.len() > 32 {
                return Err("SSH_TERMINAL_DSH_MCP_NAME_TOO_LONG".to_owned());
            }
            let patch = format!(
                "- id: mcp-{mcp_name}\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: {mcp_name}\n    transport: streamable-http\n    url: http://127.0.0.1:{}/mcp\n",
                project.remote_forward_port
            );
            format!(
                "if ! command -v dsh >/dev/null 2>&1; then printf '\033[31m[LYNX] DeepSeek Harness executable not found: dsh\033[0m\r\n'; exit 127; fi; mcp_tmp=$(mktemp) || exit 1; chmod 600 \"$mcp_tmp\"; printf '%s' {} >\"$mcp_tmp\"; trap 'rm -f \"$mcp_tmp\"' EXIT HUP INT TERM; exec dsh web --patch \"$mcp_tmp\"",
                ssh::shell_quote(&patch),
            )
        }
    };
    let command = if mode.needs_mcp() {
        format!("exec bash -lic {}", ssh::shell_quote(&command))
    } else {
        command
    };
    let mode_label = ssh::shell_quote(mode.label());
    Ok(format!(
        "if ! cd -- {sdk}; then printf '\\033[31m[LYNX] SDK directory is unavailable: %s\\033[0m\\r\\n' {sdk}; exit 72; fi; printf '\\033[90m[LYNX] managed %s session · cwd: %s · output visible in this tab\\033[0m\\r\\n' {mode_label} \"$PWD\"; export TERM=xterm-256color; {command}"
    ))
}

fn interactive_executable(host: &HostConfig, provider: AiProvider) -> Result<String, String> {
    let configured_matches = matches!(
        (&host.ai_provider, &provider),
        (AiProvider::Codex, AiProvider::Codex)
            | (AiProvider::ClaudeCode, AiProvider::ClaudeCode)
            | (AiProvider::DeepseekHarness, AiProvider::DeepseekHarness)
    );
    let executable = if configured_matches && !host.ai_executable.trim().is_empty() {
        host.ai_executable.trim()
    } else {
        provider.default_executable()
    };
    if executable.is_empty() || executable.contains('\0') {
        return Err("SSH_TERMINAL_AGENT_EXECUTABLE_INVALID".to_owned());
    }
    Ok(executable.to_owned())
}

fn validated_mcp_name(project: &ProjectConfig) -> Result<&str, String> {
    let name = project.mcp_server_name.trim();
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("SSH_TERMINAL_MCP_NAME_INVALID".to_owned());
    }
    Ok(name)
}

fn load_scope(
    state: &WorkbenchState,
    project_id: i64,
    device_id: i64,
) -> Result<(ProjectConfig, HostConfig), String> {
    let db = state.db.lock().map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    let project =
        storage::project_by_id(&db, project_id).map_err(|_| "SSH_TERMINAL_PROJECT_NOT_FOUND")?;
    if project.device_id != Some(device_id) {
        return Err("SSH_TERMINAL_SCOPE_MISMATCH".to_owned());
    }
    let host =
        storage::host_by_id(&db, project.host_id).map_err(|_| "SSH_TERMINAL_HOST_NOT_FOUND")?;
    Ok((project, host))
}

fn validate_project(project: &ProjectConfig) -> Result<(), String> {
    let sdk = project.sdk_dir.trim();
    if !sdk.starts_with('/') || sdk.contains('\0') {
        return Err("SSH_TERMINAL_SDK_DIR_INVALID".to_owned());
    }
    Ok(())
}

fn validate_size(cols: u32, rows: u32) -> Result<(), String> {
    if !(20..=500).contains(&cols) || !(5..=300).contains(&rows) {
        return Err("SSH_TERMINAL_SIZE_INVALID".to_owned());
    }
    Ok(())
}

fn ensure_snapshot_scope(
    snapshot: &SshTerminalSnapshot,
    project_id: i64,
    device_id: i64,
) -> Result<(), String> {
    if snapshot.project_id != project_id || snapshot.device_id != device_id {
        return Err("SSH_TERMINAL_SCOPE_MISMATCH".to_owned());
    }
    Ok(())
}

fn next_title(
    terminals: &std::collections::HashMap<String, SshTerminalRuntime>,
    project_id: i64,
    mode: SshTerminalMode,
) -> String {
    let count = terminals
        .values()
        .filter_map(|runtime| runtime.snapshot().ok())
        .filter(|snapshot| snapshot.project_id == project_id && snapshot.mode == mode)
        .count()
        + 1;
    format!("{}-{count}", mode.label())
}

fn append_output(
    app: &AppHandle,
    snapshot: &Arc<Mutex<SshTerminalSnapshot>>,
    terminal_id: &str,
    project_id: i64,
    device_id: i64,
    bytes: &[u8],
) {
    let (output_epoch, received_bytes) = if let Ok(mut value) = snapshot.lock() {
        value.received_bytes = value.received_bytes.saturating_add(bytes.len() as u64);
        retain_output(&mut value.data, bytes);
        (value.output_epoch, value.received_bytes)
    } else {
        return;
    };
    let _ = app.emit(
        "workbench-ssh-terminal-data",
        SshTerminalDataEvent {
            terminal_id: terminal_id.to_owned(),
            project_id,
            device_id,
            output_epoch,
            received_bytes,
            data: bytes.to_vec(),
        },
    );
}

fn increment_sent(snapshot: &Arc<Mutex<SshTerminalSnapshot>>, count: u64) {
    if let Ok(mut value) = snapshot.lock() {
        value.sent_bytes = value.sent_bytes.saturating_add(count);
    }
}

fn retain_output(retained: &mut Vec<u8>, bytes: &[u8]) {
    retained.extend_from_slice(bytes);
    if retained.len() > MAX_RETAINED_BYTES {
        let excess = retained.len() - MAX_RETAINED_BYTES;
        retained.drain(..excess);
    }
}

fn update_status(
    app: &AppHandle,
    snapshot: &Arc<Mutex<SshTerminalSnapshot>>,
    state: &str,
    message: &str,
) {
    if let Ok(mut value) = snapshot.lock() {
        value.state = state.to_owned();
        value.message = message.to_owned();
    }
    emit_status(app, snapshot);
}

fn emit_status(app: &AppHandle, snapshot: &Arc<Mutex<SshTerminalSnapshot>>) {
    let Ok(value) = snapshot.lock() else { return };
    let _ = app.emit(
        "workbench-ssh-terminal-status",
        SshTerminalStatusEvent {
            terminal_id: value.terminal_id.clone(),
            project_id: value.project_id,
            device_id: value.device_id,
            state: value.state.clone(),
            message: value.message.clone(),
            ended_at_ms: value.ended_at_ms,
            exit_code: value.exit_code,
            sent_bytes: value.sent_bytes,
            received_bytes: value.received_bytes,
        },
    );
}

fn safe_status_message(error: &str) -> String {
    let code = crate::audit::machine_error_code(error, "SSH_TERMINAL_FAILED");
    match code.as_str() {
        "HOST_KEY_CONFIRM_REQUIRED" => "SSH 主机指纹尚未确认".to_owned(),
        "HOST_KEY_CHANGED" => "SSH 主机指纹发生变化，已拒绝连接".to_owned(),
        "SSH_AUTH" => "SSH 身份验证失败".to_owned(),
        "SSH_CONNECT" => "无法连接 SSH 主机".to_owned(),
        _ => format!("远程终端已结束（{code}）"),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectConfig {
        ProjectConfig {
            id: Some(1),
            name: "T113".to_owned(),
            host_id: 2,
            device_id: Some(3),
            sdk_dir: "/opt/sdk path".to_owned(),
            build_command: String::new(),
            firmware_path: String::new(),
            local_download_dir: String::new(),
            verify_sha256: true,
            local_gateway_port: 8765,
            remote_forward_port: 18765,
            mcp_server_name: "lynx-t113".to_owned(),
            ai_provider: AiProvider::Codex,
        }
    }

    fn host() -> HostConfig {
        HostConfig {
            id: Some(2),
            name: "builder".to_owned(),
            address: "192.0.2.1".to_owned(),
            port: 22,
            username: "ubuntu".to_owned(),
            auth_type: super::super::types::SshAuthType::PrivateKey,
            private_key_path: String::new(),
            host_fingerprint: "trusted".to_owned(),
            default_work_dir: "/opt".to_owned(),
            ai_provider: AiProvider::Codex,
            ai_executable: "codex".to_owned(),
            ai_args_json: String::new(),
        }
    }

    #[test]
    fn shell_starts_in_the_saved_sdk_directory() {
        let command = terminal_command(&host(), &project(), SshTerminalMode::Shell).unwrap();
        assert!(command.contains("cd -- '/opt/sdk path'"));
        assert!(command.contains("[LYNX] managed %s session"));
        assert!(command.contains("output visible in this tab"));
        assert!(command.ends_with("exec bash -li"));
    }

    #[test]
    fn codex_uses_temporary_command_line_mcp_configuration() {
        let command = terminal_command(&host(), &project(), SshTerminalMode::Codex).unwrap();
        assert!(command.contains("mcp_servers.lynx-t113.url"));
        assert!(command.contains("127.0.0.1:18765/mcp"));
        assert!(command.contains("Codex executable not found"));
        assert!(command.contains("exec bash -lic"));
        assert!(!command.contains("~/.codex"));
    }

    #[test]
    fn claude_uses_a_mode_600_temporary_mcp_file() {
        let command = terminal_command(&host(), &project(), SshTerminalMode::ClaudeCode).unwrap();
        assert!(command.contains("mktemp"));
        assert!(command.contains("chmod 600"));
        assert!(command.contains("--mcp-config"));
        assert!(command.contains("Claude executable not found"));
        assert!(command.contains("exec bash -lic"));
        assert!(!command.contains("~/.claude"));
    }

    #[test]
    fn deepseek_harness_uses_a_scoped_temporary_cordis_patch() {
        let command =
            terminal_command(&host(), &project(), SshTerminalMode::DeepseekHarness).unwrap();
        assert!(command.contains("command -v dsh"));
        assert!(command.contains("mktemp"));
        assert!(command.contains("chmod 600"));
        assert!(command.contains("'@deepseek-ai/dsh-mcp-client'"));
        assert!(command.contains("serverName: lynx-t113"));
        assert!(command.contains("transport: streamable-http"));
        assert!(command.contains("url: http://127.0.0.1:18765/mcp"));
        assert!(command.contains("dsh web --patch"));
        assert!(command.contains("rm -f"));
        assert!(command.contains("$mcp_tmp"));
        assert!(!command.contains("~/.dsh"));
        assert!(!command.contains("cordis.yml"));
    }

    #[test]
    fn rejects_unbounded_terminal_geometry_and_unsafe_mcp_names() {
        assert!(validate_size(19, 24).is_err());
        assert!(validate_size(80, 301).is_err());
        let mut invalid = project();
        invalid.mcp_server_name = "bad.name".to_owned();
        assert!(validated_mcp_name(&invalid).is_err());
    }

    #[test]
    fn retained_output_keeps_only_the_newest_bounded_bytes() {
        let mut retained = vec![1; MAX_RETAINED_BYTES - 2];
        retain_output(&mut retained, &[2, 3, 4, 5]);
        assert_eq!(retained.len(), MAX_RETAINED_BYTES);
        assert_eq!(&retained[retained.len() - 4..], &[2, 3, 4, 5]);
    }

    #[test]
    fn terminal_snapshot_rejects_cross_project_or_cross_board_access() {
        let snapshot = SshTerminalSnapshot {
            terminal_id: "ssh-1".to_owned(),
            project_id: 10,
            device_id: 20,
            mode: SshTerminalMode::Shell,
            title: "shell-1".to_owned(),
            working_directory: "/opt/sdk".to_owned(),
            state: "connected".to_owned(),
            message: String::new(),
            started_at_ms: 0,
            ended_at_ms: None,
            exit_code: None,
            received_bytes: 0,
            sent_bytes: 0,
            output_epoch: 0,
            data: Vec::new(),
        };
        assert!(ensure_snapshot_scope(&snapshot, 10, 20).is_ok());
        assert!(ensure_snapshot_scope(&snapshot, 11, 20).is_err());
        assert!(ensure_snapshot_scope(&snapshot, 10, 21).is_err());
    }

    #[test]
    fn clear_advances_epoch_and_removes_only_retained_output() {
        let mut snapshot = SshTerminalSnapshot {
            terminal_id: "ssh-1".to_owned(),
            project_id: 10,
            device_id: 20,
            mode: SshTerminalMode::Shell,
            title: "shell-1".to_owned(),
            working_directory: "/opt/sdk".to_owned(),
            state: "connected".to_owned(),
            message: String::new(),
            started_at_ms: 0,
            ended_at_ms: None,
            exit_code: None,
            received_bytes: 42,
            sent_bytes: 12,
            output_epoch: 7,
            data: b"old output".to_vec(),
        };
        let cleared = clear_retained_output(&mut snapshot);
        assert!(snapshot.data.is_empty());
        assert_eq!(snapshot.output_epoch, 8);
        assert_eq!(cleared.output_epoch, 8);
        assert_eq!(cleared.received_bytes, 42);
    }

    #[test]
    fn terminal_input_requires_the_latest_claimed_surface() {
        assert!(
            validate_input_surface_id("workspace:550e8400-e29b-41d4-a716-446655440000").is_ok()
        );
        assert!(validate_input_surface_id("").is_err());
        assert!(validate_input_surface_id("bad surface").is_err());
        assert!(ensure_input_surface_owner(Some("surface-new"), "surface-new").is_ok());
        assert_eq!(
            ensure_input_surface_owner(Some("surface-new"), "surface-old").unwrap_err(),
            "SSH_TERMINAL_INPUT_SURFACE_STALE"
        );
        assert!(ensure_input_surface_owner(None, "surface-new").is_err());
    }
}

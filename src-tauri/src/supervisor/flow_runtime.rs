use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use super::flow::{DeviceFlow, DeviceFlowNode};
use super::types::AutomaticTask;

const RUNTIME_NODE_TYPES: &[&str] = &[
    "trigger.manual",
    "trigger.schedule",
    "trigger.webhook",
    "trigger.device",
    "trigger.serial_match",
    "serial.open",
    "serial.expect",
    "serial.send",
    "serial.login",
    "serial.command",
    "adb.wait",
    "adb.command",
    "power.action",
    "control.delay",
    "control.condition",
    "verify.deterministic",
    "result.success",
    "result.failed",
];

pub(crate) fn start_graph(
    app: &AppHandle,
    task: AutomaticTask,
    flow: DeviceFlow,
) -> Result<String, String> {
    if let Some(node) = flow
        .nodes
        .iter()
        .find(|node| !RUNTIME_NODE_TYPES.contains(&node.node_type.as_str()))
    {
        return Err(format!(
            "SUPERVISOR_DEVICE_FLOW_EXECUTOR_NOT_AVAILABLE:{}",
            node.node_type
        ));
    }

    crate::hardware_safety::ensure_idle(app, "AI 设备流程")?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let hardware_guard = safety.try_begin("AI 监督器", "设备流程", task.board_id.clone())?;
    let task_id = crate::workbench::commands::new_task_id("device-flow");
    let workbench_state = app.state::<Arc<crate::workbench::WorkbenchState>>();
    let cancellation = Arc::new(AtomicBool::new(false));
    workbench_state
        .cancellations
        .lock()
        .map_err(|error| error.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    let primary_device_id = task
        .board_id
        .strip_prefix("linux:")
        .and_then(|id| id.parse::<i64>().ok());
    {
        let db = workbench_state
            .db
            .lock()
            .map_err(|error| error.to_string())?;
        db.execute(
            "INSERT INTO task_history(task_id,kind,device_id,status,started_at,phase) \
             VALUES(?1,'device_flow',?2,'running',unixepoch(),'device_flow')",
            rusqlite::params![task_id, primary_device_id],
        )
        .map_err(|error| error.to_string())?;
    }

    let app = app.clone();
    let returned = task_id.clone();
    let workbench = workbench_state.inner().clone();
    std::thread::spawn(move || {
        let _hardware_guard = hardware_guard;
        let result = run_graph(&app, &workbench, &task_id, &flow, &cancellation);
        let (status, error) = match result {
            Ok(()) => ("success", String::new()),
            Err(error) if cancellation.load(Ordering::Relaxed) => ("cancelled", error),
            Err(error) => ("failed", error),
        };
        if let Ok(db) = workbench.db.lock() {
            let _ = crate::workbench::storage::transition_task_status(
                &db, &task_id, status, None, &error,
            );
        }
        if let Ok(mut tasks) = workbench.cancellations.lock() {
            tasks.remove(&task_id);
        }
        let _ = app.emit(
            "supervisor-automatic-result",
            serde_json::json!({"taskId": task_id, "status": status, "error": error}),
        );
    });
    Ok(returned)
}

fn run_graph(
    app: &AppHandle,
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    flow: &DeviceFlow,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    let by_id: HashMap<_, _> = flow
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut outgoing = HashMap::<&str, Vec<_>>::new();
    for edge in &flow.edges {
        outgoing.entry(edge.source.as_str()).or_default().push(edge);
    }
    let mut current = flow
        .nodes
        .iter()
        .find(|node| node.node_type.starts_with("trigger."))
        .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_TRIGGER_REQUIRED".to_owned())?;
    let serial = app.state::<Arc<crate::serial::SerialState>>();
    let mut opened_serials = FlowOpenedSerialPorts::new(serial.inner());
    let mut previous = NodeOutcome::success(String::new());

    for _ in 0..=flow.nodes.len() {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        begin_node(workbench, task_id, current)?;
        let executed = execute_node(
            app,
            workbench,
            serial.inner(),
            task_id,
            current,
            &previous,
            cancellation,
            &mut opened_serials,
        );
        finish_node(workbench, task_id, current, &executed)?;
        let outcome = match executed {
            Ok(outcome) => outcome,
            Err(error) => NodeOutcome {
                success: false,
                output: error,
            },
        };
        if current.node_type == "result.success" {
            return if previous.success {
                Ok(())
            } else {
                Err(previous.output)
            };
        }
        if current.node_type == "result.failed" {
            return Err(if previous.output.is_empty() {
                "SUPERVISOR_DEVICE_FLOW_FAILED_RESULT".to_owned()
            } else {
                previous.output
            });
        }
        let edges = outgoing
            .get(current.id.as_str())
            .cloned()
            .unwrap_or_default();
        let wanted_branch = if outcome.success {
            "success"
        } else {
            "failure"
        };
        let edge = edges
            .iter()
            .find(|edge| edge.branch.as_deref() == Some(wanted_branch))
            .or_else(|| {
                edges.iter().find(|edge| {
                    edge.branch
                        .as_deref()
                        .is_none_or(|branch| branch == "default")
                })
            })
            .ok_or_else(|| {
                if outcome.success {
                    format!("SUPERVISOR_DEVICE_FLOW_NO_NEXT_NODE:{}", current.id)
                } else {
                    format!(
                        "SUPERVISOR_DEVICE_FLOW_NODE_FAILED:{}:{}",
                        current.id, outcome.output
                    )
                }
            })?;
        current = by_id
            .get(edge.target.as_str())
            .copied()
            .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_TARGET_NOT_FOUND".to_owned())?;
        previous = outcome;
    }
    Err("SUPERVISOR_DEVICE_FLOW_STEP_LIMIT".to_owned())
}

struct FlowOpenedSerialPorts<'a> {
    serial: &'a crate::serial::SerialState,
    ports: HashSet<String>,
}

impl<'a> FlowOpenedSerialPorts<'a> {
    fn new(serial: &'a crate::serial::SerialState) -> Self {
        Self {
            serial,
            ports: HashSet::new(),
        }
    }
}

impl Drop for FlowOpenedSerialPorts<'_> {
    fn drop(&mut self) {
        for port in self.ports.drain() {
            if let Err(error) = crate::serial::commands::serial_close_core(self.serial, &port) {
                log::warn!("Failed to close device-flow serial port {port}: {error}");
            }
        }
    }
}

#[derive(Debug)]
struct NodeOutcome {
    success: bool,
    output: String,
}

impl NodeOutcome {
    fn success(output: String) -> Self {
        Self {
            success: true,
            output,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionContract {
    BoundedControl,
    SerialTransport,
    AdbTransport,
    HardwareExclusive,
    DeterministicVerifier,
}

fn execution_contract(node_type: &str) -> Option<ExecutionContract> {
    match node_type {
        "control.delay" | "control.condition" => Some(ExecutionContract::BoundedControl),
        "serial.open" | "serial.send" | "serial.expect" | "serial.command" | "serial.login" => {
            Some(ExecutionContract::SerialTransport)
        }
        "adb.wait" | "adb.command" => Some(ExecutionContract::AdbTransport),
        "power.action" => Some(ExecutionContract::HardwareExclusive),
        "verify.deterministic" => Some(ExecutionContract::DeterministicVerifier),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_node(
    app: &AppHandle,
    workbench: &crate::workbench::WorkbenchState,
    serial: &crate::serial::SerialState,
    task_id: &str,
    node: &DeviceFlowNode,
    previous: &NodeOutcome,
    cancellation: &AtomicBool,
    opened_serials: &mut FlowOpenedSerialPorts<'_>,
) -> Result<NodeOutcome, String> {
    if node.node_type.starts_with("trigger.") || node.node_type.starts_with("result.") {
        return Ok(NodeOutcome::success(previous.output.clone()));
    }
    execution_contract(&node.node_type).ok_or_else(|| {
        format!(
            "SUPERVISOR_DEVICE_FLOW_EXECUTION_CONTRACT_MISSING:{}",
            node.node_type
        )
    })?;
    match node.node_type.as_str() {
        "control.delay" => {
            sleep_cancel(config_u64(node, "delayMs", 10, 86_400_000)?, cancellation)?;
            Ok(NodeOutcome::success(previous.output.clone()))
        }
        "control.condition" => {
            let expression = node
                .config
                .get("expression")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let success = match expression.trim() {
                "previous.success == true" => previous.success,
                "previous.success == false" => !previous.success,
                _ => {
                    return Err(format!(
                        "SUPERVISOR_DEVICE_FLOW_CONDITION_UNSUPPORTED:{}",
                        node.id
                    ))
                }
            };
            Ok(NodeOutcome {
                success,
                output: previous.output.clone(),
            })
        }
        "serial.open" => {
            let (device, port) = resolved_serial(workbench, node)?;
            if !crate::serial::commands::serial_is_open_core(serial, &port) {
                crate::serial::commands::serial_open_core(
                    serial,
                    app.clone(),
                    port.clone(),
                    device.baud_rate,
                    8,
                    1,
                    "none".to_owned(),
                    "none".to_owned(),
                    Some(flow_serial_audit_context(&device, task_id)),
                )?;
                opened_serials.ports.insert(port.clone());
            }
            Ok(NodeOutcome::success(port))
        }
        "serial.send" => {
            let (device, port) = resolved_serial(workbench, node)?;
            ensure_serial_open(serial, &port)?;
            let mut text = node
                .config
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if node
                .config
                .get("appendEnter")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                text.push('\n');
            }
            flow_serial_write(app, serial, &device, &port, task_id, text.as_bytes())?;
            Ok(NodeOutcome::success(format!(
                "SERIAL_WRITE_ACCEPTED:bytes={}",
                text.len()
            )))
        }
        "serial.expect" => serial_expect(workbench, serial, node, None, cancellation),
        "serial.command" => serial_command(app, workbench, serial, task_id, node, cancellation),
        "serial.login" => serial_login(app, workbench, serial, task_id, node, cancellation),
        "adb.wait" => adb_wait(app, workbench, node, cancellation),
        "adb.command" => {
            let device = workbench_device(workbench, node)?;
            if device.adb_serial.trim().is_empty() {
                return Err("SUPERVISOR_DEVICE_FLOW_ADB_SERIAL_REQUIRED".to_owned());
            }
            let command = node
                .config
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let adb = app.state::<crate::adb::commands::AdbState>();
            let manager = adb.0.lock().map_err(|error| error.to_string())?;
            if cancellation.load(Ordering::Relaxed) {
                return Err("TASK_CANCELLED".to_owned());
            }
            let timeout = Duration::from_millis(config_u64(node, "timeoutMs", 100, 3_600_000)?);
            let marker = command_exit_marker(task_id);
            let audit_intent_id = audit_adb_command_intent(
                app,
                task_id,
                device.id.unwrap_or_default(),
                command.len(),
                timeout.as_millis() as u64,
            )?;
            let output = manager.shell_command_with_timeout(
                Some(&device.adb_serial),
                &command_with_exit_marker(command, &marker),
                timeout,
                SERIAL_FLOW_OUTPUT_LIMIT,
            );
            let outcome = output.and_then(|output| {
                if cancellation.load(Ordering::Relaxed) {
                    Err("TASK_CANCELLED".to_owned())
                } else {
                    command_outcome(&output, &marker, node)
                }
            });
            audit_adb_command_result(
                app,
                task_id,
                device.id.unwrap_or_default(),
                &audit_intent_id,
                &outcome,
            );
            outcome
        }
        "power.action" => {
            let device = workbench_device(workbench, node)?;
            let action = match node
                .config
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("get_state")
            {
                "power_on" => crate::workbench::types::PowerAction::PowerOn,
                "power_off" => crate::workbench::types::PowerAction::PowerOff,
                "boot_assert" => crate::workbench::types::PowerAction::BootAssert,
                "boot_release" => crate::workbench::types::PowerAction::BootRelease,
                "reset_pulse" => crate::workbench::types::PowerAction::ResetPulse,
                "get_state" => crate::workbench::types::PowerAction::GetState,
                _ => return Err("SUPERVISOR_DEVICE_FLOW_POWER_ACTION_INVALID".to_owned()),
            };
            let pulse = node
                .config
                .get("pulseMs")
                .and_then(Value::as_u64)
                .unwrap_or(200)
                .clamp(10, 60_000);
            let lock = workbench.device_lock(device.id.unwrap_or_default())?;
            let _device_guard = lock
                .try_lock()
                .map_err(|_| "DEVICE_BUSY:another device operation is running".to_owned())?;
            let output = crate::workbench::power::execute_and_audit(
                app,
                workbench,
                Some(task_id),
                &device,
                action,
                pulse,
                crate::workbench::power::PowerAuditActor::Supervisor,
            )?;
            Ok(NodeOutcome::success(output))
        }
        "verify.deterministic" => {
            let kind = node
                .config
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let expected = node
                .config
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let success = match kind {
                "output_match" => !expected.is_empty() && previous.output.contains(expected),
                "task_status" => expected == "success" && previous.success,
                _ => {
                    return Err(format!(
                        "SUPERVISOR_DEVICE_FLOW_VERIFIER_UNSUPPORTED:{kind}"
                    ))
                }
            };
            Ok(NodeOutcome {
                success,
                output: previous.output.clone(),
            })
        }
        _ => Err(format!(
            "SUPERVISOR_DEVICE_FLOW_EXECUTOR_NOT_AVAILABLE:{}",
            node.node_type
        )),
    }
}

fn audit_adb_command_intent(
    app: &tauri::AppHandle,
    task_id: &str,
    device_id: i64,
    command_bytes: usize,
    timeout_ms: u64,
) -> Result<String, String> {
    match crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "service".to_owned(),
            actor_id: "device-flow-runtime".to_owned(),
            category: "adb".to_owned(),
            action: "adb_command_intended".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{device_id}"),
            session_id: Some(task_id.to_owned()),
            session_kind: Some("automatic_task".to_owned()),
            session_title: Some("设备自动任务".to_owned()),
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备执行受控 ADB 命令".to_owned(),
            metadata: serde_json::json!({
                "commandByteCount": command_bytes,
                "timeoutMs": timeout_ms,
            }),
        },
    )? {
        crate::audit::AuditWriteOutcome::Recorded { event_id } => Ok(event_id),
        crate::audit::AuditWriteOutcome::Degraded { .. } => {
            Err("AUDIT_REQUIRED_DEGRADED".to_owned())
        }
    }
}

fn audit_adb_command_result(
    app: &tauri::AppHandle,
    task_id: &str,
    device_id: i64,
    causation_id: &str,
    result: &Result<NodeOutcome, String>,
) {
    let (status, error_code) = adb_command_audit_fact(result);
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "service".to_owned(),
            actor_id: "device-flow-runtime".to_owned(),
            category: "adb".to_owned(),
            action: "adb_command_completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{device_id}"),
            session_id: Some(task_id.to_owned()),
            session_kind: Some("automatic_task".to_owned()),
            session_title: Some("设备自动任务".to_owned()),
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: Some(causation_id.to_owned()),
            result: status.to_owned(),
            error_code,
            summary: if status == "success" {
                "ADB 命令执行完成"
            } else {
                "ADB 命令执行失败"
            }
            .to_owned(),
            metadata: serde_json::json!({}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn adb_command_audit_fact(result: &Result<NodeOutcome, String>) -> (&'static str, Option<String>) {
    match result {
        Ok(outcome) if outcome.success => ("success", None),
        Ok(_) => ("failed", Some("ADB_COMMAND_EXIT_NONZERO".to_owned())),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "ADB_COMMAND_FAILED",
            )),
        ),
    }
}

fn workbench_device(
    workbench: &crate::workbench::WorkbenchState,
    node: &DeviceFlowNode,
) -> Result<crate::workbench::types::DeviceConfig, String> {
    let id = node_device_id(node)?;
    let db = workbench.db.lock().map_err(|error| error.to_string())?;
    crate::workbench::storage::device_by_id(&db, id).map_err(|error| error.to_string())
}

fn node_device_id(node: &DeviceFlowNode) -> Result<i64, String> {
    node.config
        .get("boardId")
        .and_then(Value::as_str)
        .and_then(|scope| scope.strip_prefix("linux:"))
        .and_then(|id| id.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("SUPERVISOR_DEVICE_FLOW_DEVICE_REQUIRED:{}", node.id))
}

fn resolved_serial(
    workbench: &crate::workbench::WorkbenchState,
    node: &DeviceFlowNode,
) -> Result<(crate::workbench::types::DeviceConfig, String), String> {
    let device = workbench_device(workbench, node)?;
    let resolved =
        crate::serial::commands::resolve_port_core(&crate::serial::commands::SerialIdentity {
            port: device.serial_port.clone(),
            vid: device.serial_vid,
            pid: device.serial_pid,
            serial_number: device.serial_number.clone(),
            interface_number: device.serial_interface_number,
        })?;
    Ok((device, resolved.name))
}

fn serial_expect(
    workbench: &crate::workbench::WorkbenchState,
    serial: &crate::serial::SerialState,
    node: &DeviceFlowNode,
    send: Option<String>,
    cancellation: &AtomicBool,
) -> Result<NodeOutcome, String> {
    let (_, port) = resolved_serial(workbench, node)?;
    if !crate::serial::commands::serial_is_open_core(serial, &port) {
        return Err(format!("SUPERVISOR_DEVICE_FLOW_SERIAL_NOT_OPEN:{port}"));
    }
    if let Some(send) = send {
        crate::serial::commands::serial_write_core(serial, &port, send.as_bytes())?;
    }
    let expected = serial_expected(node);
    let timeout = config_u64(node, "timeoutMs", 100, 3_600_000)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        read_serial_bounded(serial, &port, &mut output)?;
        let text = String::from_utf8_lossy(&output);
        if !expected.is_empty() && text.contains(expected) {
            return Ok(NodeOutcome::success(text.into_owned()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "SUPERVISOR_DEVICE_FLOW_SERIAL_EXPECT_TIMEOUT:{}",
        node.id
    ))
}

fn serial_expected(node: &DeviceFlowNode) -> &str {
    node.config
        .get("expect")
        .or_else(|| node.config.get("pattern"))
        .or_else(|| node.config.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("#")
}

fn ensure_serial_open(serial: &crate::serial::SerialState, port: &str) -> Result<(), String> {
    if crate::serial::commands::serial_is_open_core(serial, port) {
        Ok(())
    } else {
        Err(format!("SUPERVISOR_DEVICE_FLOW_SERIAL_NOT_OPEN:{port}"))
    }
}

fn flow_serial_audit_context(
    device: &crate::workbench::types::DeviceConfig,
    task_id: &str,
) -> crate::serial::SerialAuditContext {
    crate::serial::SerialAuditContext {
        actor_type: "service".to_owned(),
        actor_id: "ai-supervisor".to_owned(),
        scope_type: "linux_board".to_owned(),
        scope_id: format!("linux:{}", device.id.unwrap_or_default()),
        session_id: task_id.to_owned(),
        session_title: "AI 监督设备流程".to_owned(),
    }
}

fn flow_serial_write(
    app: &AppHandle,
    serial: &crate::serial::SerialState,
    device: &crate::workbench::types::DeviceConfig,
    port: &str,
    task_id: &str,
    data: &[u8],
) -> Result<(), String> {
    crate::serial::commands::serial_write_audited_core(
        serial,
        port,
        data,
        app,
        &flow_serial_audit_context(device, task_id),
    )
}

fn serial_command(
    app: &AppHandle,
    workbench: &crate::workbench::WorkbenchState,
    serial: &crate::serial::SerialState,
    task_id: &str,
    node: &DeviceFlowNode,
    cancellation: &AtomicBool,
) -> Result<NodeOutcome, String> {
    let (device, port) = resolved_serial(workbench, node)?;
    ensure_serial_open(serial, &port)?;
    let timeout = config_u64(node, "timeoutMs", 100, 3_600_000)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let prompt = node
        .config
        .get("prompt")
        .or_else(|| node.config.get("expect"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("#");
    let mut output = Vec::new();

    flow_serial_write(app, serial, &device, &port, task_id, b"\n")?;
    serial_wait_for_markers(
        serial,
        &port,
        &[(prompt, SerialLoginStage::Shell)],
        deadline,
        cancellation,
        &mut output,
        node,
        "SUPERVISOR_DEVICE_FLOW_SERIAL_COMMAND_NOT_READY",
    )?;
    output.clear();

    let command = node
        .config
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let marker = command_exit_marker(task_id);
    let framed = command_with_exit_marker(command, &marker);
    flow_serial_write(
        app,
        serial,
        &device,
        &port,
        task_id,
        format!("{framed}\n").as_bytes(),
    )?;
    while Instant::now() < deadline {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        read_serial_bounded(serial, &port, &mut output)?;
        let text = String::from_utf8_lossy(&output);
        if marker_exit_code(&text, &marker).is_some() {
            return command_outcome(&text, &marker, node);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "SUPERVISOR_DEVICE_FLOW_SERIAL_COMMAND_TIMEOUT:{}",
        node.id
    ))
}

fn command_exit_marker(task_id: &str) -> String {
    let safe: String = task_id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .take(48)
        .collect();
    format!("__LYNX_EXIT_{safe}__")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn command_with_exit_marker(command: &str, marker: &str) -> String {
    format!(
        "sh -c {}; lynx_status=$?; printf '\\n{}%s\\n' \"$lynx_status\"",
        shell_quote(command),
        marker
    )
}

fn marker_exit_code(output: &str, marker: &str) -> Option<(usize, i32)> {
    let position = output.rfind(marker)?;
    let tail = &output[position + marker.len()..];
    let line = tail.split_once('\n')?.0.trim_matches('\r').trim();
    line.parse::<i32>().ok().map(|code| (position, code))
}

fn command_outcome(
    output: &str,
    marker: &str,
    node: &DeviceFlowNode,
) -> Result<NodeOutcome, String> {
    let (position, exit_code) = marker_exit_code(output, marker).ok_or_else(|| {
        format!(
            "SUPERVISOR_DEVICE_FLOW_COMMAND_EXIT_STATUS_MISSING:{}",
            node.id
        )
    })?;
    let framed = command_with_exit_marker(
        node.config
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        marker,
    );
    let clean = output[..position]
        .lines()
        .filter(|line| !line.contains(&framed))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned();
    if exit_code == 0 {
        Ok(NodeOutcome::success(clean))
    } else {
        Ok(NodeOutcome {
            success: false,
            output: format!("COMMAND_EXIT_CODE:{exit_code}\n{clean}"),
        })
    }
}

const SERIAL_FLOW_OUTPUT_LIMIT: usize = 512 * 1024;

fn read_serial_bounded(
    serial: &crate::serial::SerialState,
    port: &str,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let chunk = crate::serial::commands::serial_read_core(serial, port, 65_536)?;
    let excess = output
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(SERIAL_FLOW_OUTPUT_LIMIT);
    if excess > 0 {
        output.drain(..excess.min(output.len()));
    }
    output.extend(chunk);
    Ok(())
}

fn serial_login(
    app: &AppHandle,
    workbench: &crate::workbench::WorkbenchState,
    serial: &crate::serial::SerialState,
    task_id: &str,
    node: &DeviceFlowNode,
    cancellation: &AtomicBool,
) -> Result<NodeOutcome, String> {
    let (device, port) = resolved_serial(workbench, node)?;
    if !crate::serial::commands::serial_is_open_core(serial, &port) {
        return Err(format!("SUPERVISOR_DEVICE_FLOW_SERIAL_NOT_OPEN:{port}"));
    }
    let login_prompt = serial_login_marker(node, "loginPrompt", "login:");
    let password_prompt = serial_login_marker(node, "passwordPrompt", "Password:");
    let shell_prompt = serial_expected(node).to_owned();
    let timeout = config_u64(node, "timeoutMs", 100, 3_600_000)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let mut output = Vec::new();

    // Wake a quiet terminal first. Do not send the username until the target
    // has explicitly presented a login prompt; boot logs may otherwise consume
    // credentials as ordinary console input.
    flow_serial_write(app, serial, &device, &port, task_id, b"\n")?;
    match serial_wait_for_markers(
        serial,
        &port,
        &[
            (login_prompt, SerialLoginStage::Login),
            (&shell_prompt, SerialLoginStage::Shell),
        ],
        deadline,
        cancellation,
        &mut output,
        node,
        "SUPERVISOR_DEVICE_FLOW_SERIAL_LOGIN_TIMEOUT",
    )? {
        SerialLoginStage::Shell => {
            return Ok(NodeOutcome::success(
                String::from_utf8_lossy(&output).into_owned(),
            ));
        }
        SerialLoginStage::Login => {}
        SerialLoginStage::Password => unreachable!("password is not an initial login marker"),
    }

    let username = node
        .config
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("root");
    flow_serial_write(
        app,
        serial,
        &device,
        &port,
        task_id,
        format!("{username}\n").as_bytes(),
    )?;
    let credential_reference = node
        .config
        .get("credentialRef")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let next_stage = serial_wait_for_markers(
        serial,
        &port,
        &[
            (password_prompt, SerialLoginStage::Password),
            (&shell_prompt, SerialLoginStage::Shell),
        ],
        deadline,
        cancellation,
        &mut output,
        node,
        "SUPERVISOR_DEVICE_FLOW_SERIAL_LOGIN_TIMEOUT",
    )?;
    if next_stage == SerialLoginStage::Shell {
        return Ok(NodeOutcome::success(
            String::from_utf8_lossy(&output).into_owned(),
        ));
    }
    let Some(reference) = credential_reference else {
        return Err(format!(
            "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_REQUIRED:{}",
            node.id
        ));
    };
    debug_assert_eq!(next_stage, SerialLoginStage::Password);
    let password = keyring::Entry::new(&crate::keyring_service("dshanpi-lynx-device-flow"), reference)
        .map_err(|_| "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_UNAVAILABLE".to_owned())?
        .get_password()
        .map_err(|_| "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_UNAVAILABLE".to_owned())?;
    flow_serial_write(
        app,
        serial,
        &device,
        &port,
        task_id,
        format!("{password}\n").as_bytes(),
    )?;
    serial_wait_for_markers(
        serial,
        &port,
        &[(&shell_prompt, SerialLoginStage::Shell)],
        deadline,
        cancellation,
        &mut output,
        node,
        "SUPERVISOR_DEVICE_FLOW_SERIAL_LOGIN_TIMEOUT",
    )?;
    let safe_output = String::from_utf8_lossy(&output).replace(&password, "[REDACTED]");
    Ok(NodeOutcome::success(safe_output))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SerialLoginStage {
    Login,
    Password,
    Shell,
}

fn serial_login_marker<'a>(node: &'a DeviceFlowNode, key: &str, fallback: &'a str) -> &'a str {
    node.config
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
}

#[allow(clippy::too_many_arguments)]
fn serial_wait_for_markers(
    serial: &crate::serial::SerialState,
    port: &str,
    markers: &[(&str, SerialLoginStage)],
    deadline: Instant,
    cancellation: &AtomicBool,
    output: &mut Vec<u8>,
    node: &DeviceFlowNode,
    timeout_code: &str,
) -> Result<SerialLoginStage, String> {
    while Instant::now() < deadline {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        read_serial_bounded(serial, port, output)?;
        let text = String::from_utf8_lossy(output);
        if let Some(stage) = matching_login_stage(&text, markers) {
            return Ok(stage);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("{timeout_code}:{}", node.id))
}

fn matching_login_stage(
    text: &str,
    markers: &[(&str, SerialLoginStage)],
) -> Option<SerialLoginStage> {
    markers
        .iter()
        .filter_map(|(marker, stage)| {
            if marker.is_empty() {
                None
            } else {
                text.rfind(marker).map(|position| (position, *stage))
            }
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, stage)| stage)
}

fn adb_wait(
    app: &AppHandle,
    workbench: &crate::workbench::WorkbenchState,
    node: &DeviceFlowNode,
    cancellation: &AtomicBool,
) -> Result<NodeOutcome, String> {
    let device = workbench_device(workbench, node)?;
    if device.adb_serial.trim().is_empty() {
        return Err("SUPERVISOR_DEVICE_FLOW_ADB_SERIAL_REQUIRED".to_owned());
    }
    let timeout = config_u64(node, "timeoutMs", 100, 3_600_000)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    while Instant::now() < deadline {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        let adb = app.state::<crate::adb::commands::AdbState>();
        if adb
            .0
            .lock()
            .map_err(|error| error.to_string())?
            .list_devices()?
            .iter()
            .any(|item| item.serial == device.adb_serial)
        {
            return Ok(NodeOutcome::success(device.adb_serial));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("SUPERVISOR_DEVICE_FLOW_ADB_WAIT_TIMEOUT".to_owned())
}

fn config_u64(node: &DeviceFlowNode, key: &str, min: u64, max: u64) -> Result<u64, String> {
    node.config
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value >= min && *value <= max)
        .ok_or_else(|| format!("SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}", node.id))
}

fn sleep_cancel(milliseconds: u64, cancellation: &AtomicBool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < deadline {
        if cancellation.load(Ordering::Relaxed) {
            return Err("TASK_CANCELLED".to_owned());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

fn begin_node(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    node: &DeviceFlowNode,
) -> Result<(), String> {
    let db = workbench.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO workflow_steps(task_id,step_name,status,started_at) \
         VALUES(?1,?2,'running',unixepoch())",
        rusqlite::params![task_id, format!("{}:{}", node.id, node.label)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn finish_node(
    workbench: &crate::workbench::WorkbenchState,
    task_id: &str,
    node: &DeviceFlowNode,
    result: &Result<NodeOutcome, String>,
) -> Result<(), String> {
    let (status, error) = match result {
        Ok(outcome) if outcome.success => ("success", ""),
        Ok(outcome) => ("failed", outcome.output.as_str()),
        Err(error) => ("failed", error.as_str()),
    };
    let db = workbench.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE workflow_steps SET status=?1,finished_at=unixepoch(),error=?2 \
         WHERE id=(SELECT id FROM workflow_steps WHERE task_id=?3 AND step_name=?4 \
         ORDER BY id DESC LIMIT 1)",
        rusqlite::params![
            status,
            error,
            task_id,
            format!("{}:{}", node.id, node.label)
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(config: Value) -> DeviceFlowNode {
        DeviceFlowNode {
            id: "login".to_owned(),
            node_type: "serial.login".to_owned(),
            label: "登录".to_owned(),
            x: 0.0,
            y: 0.0,
            config,
        }
    }

    #[test]
    fn serial_login_uses_the_configured_shell_prompt() {
        assert_eq!(
            serial_expected(&node(serde_json::json!({"prompt":"lynx>"}))),
            "lynx>"
        );
        assert_eq!(serial_expected(&node(serde_json::json!({}))), "#");
    }

    #[test]
    fn serial_login_uses_distinct_configurable_stage_markers() {
        let configured = node(serde_json::json!({
            "loginPrompt": "user:",
            "passwordPrompt": "secret:",
            "prompt": "lynx>"
        }));
        assert_eq!(
            serial_login_marker(&configured, "loginPrompt", "login:"),
            "user:"
        );
        assert_eq!(
            serial_login_marker(&configured, "passwordPrompt", "Password:"),
            "secret:"
        );
        assert_eq!(
            matching_login_stage(
                "boot complete\nuser:",
                &[
                    ("user:", SerialLoginStage::Login),
                    ("lynx>", SerialLoginStage::Shell)
                ]
            ),
            Some(SerialLoginStage::Login)
        );
        assert_eq!(
            matching_login_stage("lynx>", &[("lynx>", SerialLoginStage::Shell)]),
            Some(SerialLoginStage::Shell)
        );
        assert_eq!(
            matching_login_stage(
                "device login:\nlynx>",
                &[
                    ("login:", SerialLoginStage::Login),
                    ("lynx>", SerialLoginStage::Shell)
                ]
            ),
            Some(SerialLoginStage::Shell)
        );
    }

    #[test]
    fn command_completion_requires_a_real_exit_marker_and_preserves_exit_code() {
        assert_eq!(shell_quote("echo 'ok'"), "'echo '\"'\"'ok'\"'\"''");
        let marker = command_exit_marker("device-flow-123");
        let framed = command_with_exit_marker("false", &marker);
        assert!(framed.contains("sh -c 'false'"));
        assert_eq!(
            marker_exit_code(&format!("output\n{marker}0\n"), &marker),
            Some((7, 0))
        );
        let failed = command_outcome(
            &format!(
                "{}\nbad\n{marker}7\n",
                command_with_exit_marker("", &marker)
            ),
            &marker,
            &node(serde_json::json!({})),
        )
        .unwrap();
        assert!(!failed.success);
        assert!(failed.output.starts_with("COMMAND_EXIT_CODE:7"));
        assert!(!failed.output.contains("sh -c"));
        assert!(command_outcome("no marker", &marker, &node(serde_json::json!({}))).is_err());
    }

    #[test]
    fn adb_audit_result_uses_the_remote_exit_status() {
        assert_eq!(
            adb_command_audit_fact(&Ok(NodeOutcome::success(String::new()))),
            ("success", None)
        );
        assert_eq!(
            adb_command_audit_fact(&Ok(NodeOutcome {
                success: false,
                output: "COMMAND_EXIT_CODE:7".to_owned(),
            })),
            ("failed", Some("ADB_COMMAND_EXIT_NONZERO".to_owned()))
        );
        assert_eq!(
            adb_command_audit_fact(&Err("ADB_TIMEOUT".to_owned())),
            ("failed", Some("ADB_TIMEOUT".to_owned()))
        );
    }

    #[test]
    fn every_runtime_action_has_an_explicit_execution_contract() {
        for node_type in RUNTIME_NODE_TYPES.iter().filter(|node_type| {
            !node_type.starts_with("trigger.") && !node_type.starts_with("result.")
        }) {
            assert!(
                execution_contract(node_type).is_some(),
                "missing execution contract for {node_type}"
            );
        }
    }
}

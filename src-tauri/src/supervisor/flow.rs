use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::types::AutomaticTask;

const ALLOWED_NODE_TYPES: &[&str] = &[
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
    "flash.workbench",
    "ssh.build",
    "mcu.operation",
    "control.delay",
    "control.condition",
    "control.retry",
    "supervisor.analyze",
    "supervisor.human",
    "verify.deterministic",
    "result.success",
    "result.failed",
];

const HIGH_RISK_NODE_TYPES: &[&str] = &[
    "serial.send",
    "serial.command",
    "adb.command",
    "power.action",
    "flash.workbench",
    "ssh.build",
    "mcu.operation",
];

const DRAFT_ONLY_NODE_TYPES: &[&str] = &[
    "trigger.serial_match",
    "ssh.build",
    "mcu.operation",
    "control.retry",
    "supervisor.analyze",
    "supervisor.human",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceFlow {
    pub kind: String,
    pub version: u32,
    pub nodes: Vec<DeviceFlowNode>,
    pub edges: Vec<DeviceFlowEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceFlowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceFlowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub branch: Option<String>,
}

pub(crate) fn is_device_flow(task: &AutomaticTask) -> bool {
    task.task_template.get("kind").and_then(Value::as_str) == Some("device_flow")
}

pub(crate) fn parse(task: &AutomaticTask) -> Result<DeviceFlow, String> {
    serde_json::from_value(task.task_template.clone())
        .map_err(|_| "SUPERVISOR_DEVICE_FLOW_SCHEMA_INVALID".to_owned())
}

/// Validate a graph that is being persisted as a disabled work-in-progress.
/// Drafts may be incomplete or disconnected, but their schema, identities,
/// secrets and any edges already present must still be safe and unambiguous.
pub(crate) fn validate_draft(task: &AutomaticTask) -> Result<Vec<String>, String> {
    let flow = parse(task)?;
    if flow.kind != "device_flow" || flow.version != 1 {
        return Err("SUPERVISOR_DEVICE_FLOW_VERSION_INVALID".to_owned());
    }
    if flow.nodes.len() > 200 || flow.edges.len() > 400 {
        return Err("SUPERVISOR_DEVICE_FLOW_SIZE_LIMIT".to_owned());
    }
    let mut ids = HashSet::new();
    let mut scopes = HashSet::new();
    for node in &flow.nodes {
        if node.id.trim().is_empty()
            || node.id.len() > 128
            || node.label.trim().is_empty()
            || node.label.len() > 200
            || !node.x.is_finite()
            || !node.y.is_finite()
            || node.x < 0.0
            || node.y < 0.0
            || !ids.insert(node.id.clone())
        {
            return Err("SUPERVISOR_DEVICE_FLOW_NODE_IDENTITY_INVALID".to_owned());
        }
        if !ALLOWED_NODE_TYPES.contains(&node.node_type.as_str()) {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_NODE_TYPE_UNSUPPORTED:{}",
                node.node_type
            ));
        }
        if !node.config.is_object() || contains_plaintext_secret(&node.config) {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_NODE_CONFIG_OR_SECRET_INVALID:{}",
                node.id
            ));
        }
        if HIGH_RISK_NODE_TYPES.contains(&node.node_type.as_str())
            && node.config.get("approvalMode").and_then(Value::as_str) != Some("required")
        {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_APPROVAL_REQUIRED:{}",
                node.id
            ));
        }
        if requires_linux_scope(&node.node_type) || node.node_type == "mcu.operation" {
            if let Some(board_id) = node.config.get("boardId").and_then(Value::as_str) {
                if !valid_board_scope(board_id)
                    || (node.node_type == "mcu.operation" && !board_id.starts_with("mcu:"))
                    || (requires_linux_scope(&node.node_type) && !board_id.starts_with("linux:"))
                {
                    return Err(format!(
                        "SUPERVISOR_DEVICE_FLOW_BOARD_SCOPE_INVALID:{}",
                        node.id
                    ));
                }
                scopes.insert(board_id.to_owned());
            }
        }
    }

    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut outgoing_slots = HashSet::new();
    let mut single_outgoing = HashSet::new();
    let mut edge_ids = HashSet::new();
    for edge in &flow.edges {
        if edge.id.trim().is_empty()
            || !edge_ids.insert(edge.id.as_str())
            || !ids.contains(&edge.source)
            || !ids.contains(&edge.target)
            || edge.source == edge.target
            || edge
                .branch
                .as_deref()
                .is_some_and(|branch| !matches!(branch, "success" | "failure" | "default"))
        {
            return Err("SUPERVISOR_DEVICE_FLOW_EDGE_INVALID".to_owned());
        }
        let source = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_EDGE_INVALID".to_owned())?;
        let target = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_EDGE_INVALID".to_owned())?;
        if source.node_type.starts_with("result.") || target.node_type.starts_with("trigger.") {
            return Err("SUPERVISOR_DEVICE_FLOW_TERMINAL_DIRECTION_INVALID".to_owned());
        }
        if matches!(
            source.node_type.as_str(),
            "verify.deterministic" | "control.condition"
        ) {
            if !outgoing_slots.insert((
                edge.source.as_str(),
                edge.branch.as_deref().unwrap_or("default"),
            )) {
                return Err("SUPERVISOR_DEVICE_FLOW_BRANCH_AMBIGUOUS".to_owned());
            }
        } else if !single_outgoing.insert(edge.source.as_str()) {
            return Err("SUPERVISOR_DEVICE_FLOW_MULTIPLE_OUTGOING".to_owned());
        }
        outgoing
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for node in &flow.nodes {
        if has_cycle(&node.id, &outgoing, &mut visiting, &mut visited) {
            return Err("SUPERVISOR_DEVICE_FLOW_UNBOUNDED_CYCLE".to_owned());
        }
    }
    let mut scopes: Vec<_> = scopes.into_iter().collect();
    scopes.sort();
    Ok(scopes)
}

pub(crate) fn validate(task: &AutomaticTask) -> Result<Vec<String>, String> {
    let flow = parse(task)?;
    if flow.kind != "device_flow" || flow.version != 1 || flow.nodes.is_empty() {
        return Err("SUPERVISOR_DEVICE_FLOW_VERSION_OR_NODES_INVALID".to_owned());
    }
    if flow.nodes.len() > 200 || flow.edges.len() > 400 {
        return Err("SUPERVISOR_DEVICE_FLOW_SIZE_LIMIT".to_owned());
    }
    let mut ids = HashSet::new();
    let mut scopes = HashSet::new();
    let mut trigger_count = 0;
    let mut result_count = 0;
    let mut verifier_count = 0;
    for node in &flow.nodes {
        if node.id.trim().is_empty()
            || node.id.len() > 128
            || node.label.trim().is_empty()
            || node.label.len() > 200
            || !node.x.is_finite()
            || !node.y.is_finite()
            || node.x < 0.0
            || node.y < 0.0
            || !ids.insert(node.id.clone())
        {
            return Err("SUPERVISOR_DEVICE_FLOW_NODE_IDENTITY_INVALID".to_owned());
        }
        if !ALLOWED_NODE_TYPES.contains(&node.node_type.as_str()) {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_NODE_TYPE_UNSUPPORTED:{}",
                node.node_type
            ));
        }
        if task.enabled && DRAFT_ONLY_NODE_TYPES.contains(&node.node_type.as_str()) {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_EXECUTOR_NOT_AVAILABLE:{}",
                node.node_type
            ));
        }
        if !node.config.is_object() || contains_plaintext_secret(&node.config) {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_NODE_CONFIG_OR_SECRET_INVALID:{}",
                node.id
            ));
        }
        if node.node_type.starts_with("trigger.") {
            trigger_count += 1;
        }
        if node.node_type.starts_with("result.") {
            result_count += 1;
        }
        if node.node_type == "verify.deterministic" {
            verifier_count += 1;
        }
        if HIGH_RISK_NODE_TYPES.contains(&node.node_type.as_str())
            && node.config.get("approvalMode").and_then(Value::as_str) != Some("required")
        {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_APPROVAL_REQUIRED:{}",
                node.id
            ));
        }
        if requires_linux_scope(&node.node_type) || node.node_type == "mcu.operation" {
            let board_id = node
                .config
                .get("boardId")
                .and_then(Value::as_str)
                .filter(|value| valid_board_scope(value))
                .ok_or_else(|| format!("SUPERVISOR_DEVICE_FLOW_BOARD_REQUIRED:{}", node.id))?;
            if node.node_type == "mcu.operation" && !board_id.starts_with("mcu:") {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_MCU_SCOPE_INVALID:{}",
                    node.id
                ));
            }
            if requires_linux_scope(&node.node_type) && !board_id.starts_with("linux:") {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_LINUX_SCOPE_INVALID:{}",
                    node.id
                ));
            }
            scopes.insert(board_id.to_owned());
        }
        validate_node_bounds(node)?;
    }
    if trigger_count != 1 || result_count == 0 || verifier_count == 0 {
        return Err("SUPERVISOR_DEVICE_FLOW_TRIGGER_RESULT_VERIFY_REQUIRED".to_owned());
    }
    if scopes.is_empty() || !scopes.contains(&task.board_id) {
        return Err("SUPERVISOR_DEVICE_FLOW_PRIMARY_BOARD_SCOPE_MISMATCH".to_owned());
    }

    let executable: Vec<_> = flow
        .nodes
        .iter()
        .filter(|node| {
            !node.node_type.starts_with("trigger.")
                && !node.node_type.starts_with("result.")
                && node.node_type != "verify.deterministic"
        })
        .collect();
    if task.enabled
        && executable
            .iter()
            .any(|node| node.node_type == "flash.workbench")
        && (executable.len() != 1 || executable[0].node_type != "flash.workbench")
    {
        return Err("SUPERVISOR_DEVICE_FLOW_FLASH_COMPOSITION_UNSUPPORTED".to_owned());
    }

    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut outgoing_slots = HashSet::new();
    let mut single_outgoing = HashSet::new();
    let mut connected = HashSet::new();
    let mut edge_ids = HashSet::new();
    for edge in &flow.edges {
        if edge.id.trim().is_empty()
            || !edge_ids.insert(edge.id.as_str())
            || !ids.contains(&edge.source)
            || !ids.contains(&edge.target)
            || edge.source == edge.target
            || edge
                .branch
                .as_deref()
                .is_some_and(|branch| !matches!(branch, "success" | "failure" | "default"))
        {
            return Err("SUPERVISOR_DEVICE_FLOW_EDGE_INVALID".to_owned());
        }
        let source = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_EDGE_INVALID".to_owned())?;
        if matches!(
            source.node_type.as_str(),
            "verify.deterministic" | "control.condition"
        ) {
            if !outgoing_slots.insert((
                edge.source.as_str(),
                edge.branch.as_deref().unwrap_or("default"),
            )) {
                return Err("SUPERVISOR_DEVICE_FLOW_BRANCH_AMBIGUOUS".to_owned());
            }
        } else if !single_outgoing.insert(edge.source.as_str()) {
            return Err("SUPERVISOR_DEVICE_FLOW_MULTIPLE_OUTGOING".to_owned());
        }
        outgoing
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        connected.insert(edge.source.as_str());
        connected.insert(edge.target.as_str());
    }
    let trigger = flow
        .nodes
        .iter()
        .find(|node| node.node_type.starts_with("trigger."))
        .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_TRIGGER_REQUIRED".to_owned())?;
    if flow.nodes.len() > 1
        && flow
            .nodes
            .iter()
            .any(|node| !connected.contains(node.id.as_str()))
    {
        return Err("SUPERVISOR_DEVICE_FLOW_DISCONNECTED_NODE".to_owned());
    }
    let mut reachable = HashSet::new();
    let mut pending = vec![trigger.id.as_str()];
    while let Some(id) = pending.pop() {
        if reachable.insert(id) {
            pending.extend(
                (outgoing.get(id).cloned().unwrap_or_default())
                    .iter()
                    .copied(),
            );
        }
    }
    if flow
        .nodes
        .iter()
        .any(|node| !reachable.contains(node.id.as_str()))
    {
        return Err("SUPERVISOR_DEVICE_FLOW_UNREACHABLE_NODE".to_owned());
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for node in &flow.nodes {
        if has_cycle(&node.id, &outgoing, &mut visiting, &mut visited) {
            return Err("SUPERVISOR_DEVICE_FLOW_UNBOUNDED_CYCLE".to_owned());
        }
    }
    if flow.edges.iter().any(|edge| edge.target == trigger.id)
        || flow.nodes.iter().any(|node| {
            node.node_type.starts_with("result.")
                && flow.edges.iter().any(|edge| edge.source == node.id)
        })
    {
        return Err("SUPERVISOR_DEVICE_FLOW_TERMINAL_DIRECTION_INVALID".to_owned());
    }
    let mut scopes: Vec<_> = scopes.into_iter().collect();
    scopes.sort();
    Ok(scopes)
}

/// Compile the first executable device-flow shape onto an existing, fully
/// guarded Workbench workflow. Other node types remain saveable as drafts but
/// are rejected at execution instead of being simulated or silently skipped.
pub(crate) fn start(
    app: &AppHandle,
    task: AutomaticTask,
    operator_approved: bool,
) -> Result<String, String> {
    validate(&task)?;
    let flow = parse(&task)?;
    if !operator_approved
        && flow
            .nodes
            .iter()
            .any(|node| HIGH_RISK_NODE_TYPES.contains(&node.node_type.as_str()))
    {
        return Err("SUPERVISOR_DEVICE_FLOW_OPERATOR_APPROVAL_REQUIRED".to_owned());
    }
    let executable: Vec<_> = flow
        .nodes
        .iter()
        .filter(|node| {
            !node.node_type.starts_with("trigger.")
                && !node.node_type.starts_with("result.")
                && node.node_type != "verify.deterministic"
        })
        .collect();
    if executable.len() != 1 || executable[0].node_type != "flash.workbench" {
        return super::flow_runtime::start_graph(app, task, flow);
    }
    let node = executable[0];
    let project_id = json_number(&node.config, "project_id", "projectId")
        .filter(|id| *id > 0)
        .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_PROJECT_REQUIRED".to_owned())?;
    let device_id = node
        .config
        .get("boardId")
        .and_then(Value::as_str)
        .and_then(|scope| scope.strip_prefix("linux:"))
        .and_then(|id| id.parse::<i64>().ok())
        .or_else(|| json_number(&node.config, "device_id", "deviceId"))
        .filter(|id| *id > 0)
        .ok_or_else(|| "SUPERVISOR_DEVICE_FLOW_DEVICE_REQUIRED".to_owned())?;
    if task.board_id != format!("linux:{device_id}") {
        return Err("SUPERVISOR_DEVICE_FLOW_PRIMARY_BOARD_SCOPE_MISMATCH".to_owned());
    }
    let workbench = app.state::<Arc<crate::workbench::WorkbenchState>>();
    let serial = app.state::<Arc<crate::serial::SerialState>>();
    crate::workbench::workflow::start_workflow(
        project_id,
        device_id,
        crate::workbench::types::SshCredentials::default(),
        app.clone(),
        workbench.inner(),
        serial.inner(),
    )
}

fn json_number(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(Value::as_i64)
}

fn requires_linux_scope(node_type: &str) -> bool {
    node_type.starts_with("serial.")
        || node_type.starts_with("adb.")
        || matches!(
            node_type,
            "trigger.serial_match" | "power.action" | "flash.workbench" | "ssh.build"
        )
}

fn valid_board_scope(value: &str) -> bool {
    let Some((domain, id)) = value.split_once(':') else {
        return false;
    };
    match domain {
        "linux" => id.parse::<i64>().is_ok_and(|id| id > 0),
        "mcu" => {
            !id.is_empty()
                && id.len() <= 96
                && id
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
        }
        _ => false,
    }
}

fn contains_plaintext_secret(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = key.to_ascii_lowercase();
            (!normalized.ends_with("ref")
                && !normalized.ends_with("prompt")
                && (normalized.contains("password")
                    || normalized.contains("secret")
                    || normalized.contains("token")
                    || normalized.contains("privatekey")
                    || normalized.contains("authorization")))
                || contains_plaintext_secret(value)
        }),
        Value::Array(values) => values.iter().any(contains_plaintext_secret),
        _ => false,
    }
}

fn validate_node_bounds(node: &DeviceFlowNode) -> Result<(), String> {
    let number = |key: &str| node.config.get(key).and_then(Value::as_i64);
    if number("timeoutMs").is_some_and(|value| !(100..=3_600_000).contains(&value))
        || number("delayMs").is_some_and(|value| !(10..=86_400_000).contains(&value))
        || number("attempts").is_some_and(|value| !(1..=20).contains(&value))
        || number("pulseMs").is_some_and(|value| !(10..=60_000).contains(&value))
    {
        return Err(format!(
            "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
            node.id
        ));
    }
    if matches!(node.node_type.as_str(), "serial.command" | "adb.command") {
        let command = node
            .config
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if command.trim().is_empty() || command.len() > 4096 || command.contains('\0') {
            return Err(format!(
                "SUPERVISOR_DEVICE_FLOW_COMMAND_INVALID:{}",
                node.id
            ));
        }
    }
    let required_text = |key: &str, max: usize| {
        node.config
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.len() <= max && !value.contains('\0'))
            .ok_or_else(|| format!("SUPERVISOR_DEVICE_FLOW_NODE_TEXT_INVALID:{}:{key}", node.id))
    };
    match node.node_type.as_str() {
        "serial.send" => {
            required_text("text", 65_536)?;
        }
        "serial.expect" => {
            required_text("pattern", 4_096)?;
            if number("timeoutMs").is_none() {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
                    node.id
                ));
            }
        }
        "serial.login" => {
            required_text("username", 256)?;
            required_text("prompt", 256)?;
            if number("timeoutMs").is_none() {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
                    node.id
                ));
            }
        }
        "serial.command" => {
            node.config
                .get("prompt")
                .or_else(|| node.config.get("expect"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 256)
                .ok_or_else(|| {
                    format!(
                        "SUPERVISOR_DEVICE_FLOW_NODE_TEXT_INVALID:{}:prompt",
                        node.id
                    )
                })?;
            if number("timeoutMs").is_none() {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
                    node.id
                ));
            }
        }
        "adb.wait" | "adb.command" => {
            if number("timeoutMs").is_none() {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
                    node.id
                ));
            }
        }
        "control.delay" => {
            if number("delayMs").is_none() {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_NODE_BUDGET_INVALID:{}",
                    node.id
                ));
            }
        }
        "control.condition" => {
            let expression = required_text("expression", 128)?;
            if !matches!(
                expression,
                "previous.success == true" | "previous.success == false"
            ) {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_CONDITION_UNSUPPORTED:{}",
                    node.id
                ));
            }
        }
        "power.action" => {
            let action = required_text("action", 32)?;
            if !matches!(
                action,
                "power_on"
                    | "power_off"
                    | "boot_assert"
                    | "boot_release"
                    | "reset_pulse"
                    | "get_state"
            ) {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_POWER_ACTION_INVALID:{}",
                    node.id
                ));
            }
        }
        "verify.deterministic" => {
            let kind = required_text("kind", 64)?;
            let pattern = required_text("pattern", 4_096)?;
            if !matches!(kind, "output_match" | "task_status")
                || (kind == "task_status" && pattern != "success")
            {
                return Err(format!(
                    "SUPERVISOR_DEVICE_FLOW_VERIFIER_INVALID:{}",
                    node.id
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn has_cycle<'a>(
    node: &'a str,
    outgoing: &HashMap<&'a str, Vec<&'a str>>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
) -> bool {
    if visiting.contains(node) {
        return true;
    }
    if visited.contains(node) {
        return false;
    }
    visiting.insert(node);
    if outgoing.get(node).is_some_and(|targets| {
        targets
            .iter()
            .any(|target| has_cycle(target, outgoing, visiting, visited))
    }) {
        return true;
    }
    visiting.remove(node);
    visited.insert(node);
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn task(template: Value) -> AutomaticTask {
        AutomaticTask {
            id: None,
            name: "device flow".to_owned(),
            enabled: false,
            trigger_type: "manual_api".to_owned(),
            trigger: json!({}),
            board_id: "linux:4".to_owned(),
            task_template: template,
            policy_id: Some(1),
            success_conditions: vec![json!({"kind":"exit_code","equals":0})],
            next_run_at: None,
            last_run_at: None,
        }
    }

    fn valid_template() -> Value {
        json!({
            "kind":"device_flow", "version":1,
            "nodes":[
                {"id":"a","type":"trigger.webhook","label":"start","x":0,"y":0,"config":{}},
                {"id":"b","type":"adb.command","label":"check","x":10,"y":0,"config":{"boardId":"linux:4","command":"getprop ro.build.version.release","timeoutMs":1000,"approvalMode":"required"}},
                {"id":"c","type":"verify.deterministic","label":"verify","x":20,"y":0,"config":{"kind":"output_match","pattern":"14"}},
                {"id":"d","type":"result.success","label":"done","x":30,"y":0,"config":{}}
            ],
            "edges":[
                {"id":"ab","source":"a","target":"b"},
                {"id":"bc","source":"b","target":"c"},
                {"id":"cd","source":"c","target":"d","branch":"success"}
            ]
        })
    }

    #[test]
    fn valid_device_flow_preserves_fixed_scope() {
        assert_eq!(validate(&task(valid_template())).unwrap(), vec!["linux:4"]);
    }

    #[test]
    fn disabled_draft_can_be_empty_or_disconnected_without_weakening_edge_safety() {
        let empty = json!({"kind":"device_flow", "version":1, "nodes":[], "edges":[]});
        assert!(validate_draft(&task(empty)).unwrap().is_empty());

        let mut disconnected = valid_template();
        disconnected["edges"] = json!([]);
        assert_eq!(
            validate_draft(&task(disconnected)).unwrap(),
            vec!["linux:4"]
        );

        let mut unsafe_edge = valid_template();
        unsafe_edge["edges"] = json!([{"id":"da","source":"d","target":"a"}]);
        assert_eq!(
            validate_draft(&task(unsafe_edge)).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_TERMINAL_DIRECTION_INVALID"
        );
    }

    #[test]
    fn device_flow_rejects_cycles_plaintext_secrets_and_missing_approval() {
        let mut template = valid_template();
        template["nodes"][1]["config"]["password"] = json!("plaintext");
        assert!(validate(&task(template)).unwrap_err().contains("SECRET"));
        let mut template = valid_template();
        template["nodes"][1]["config"]
            .as_object_mut()
            .unwrap()
            .remove("approvalMode");
        assert!(validate(&task(template)).unwrap_err().contains("APPROVAL"));
        let mut template = valid_template();
        template["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"da","source":"d","target":"a"}));
        assert_eq!(
            validate(&task(template)).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_UNBOUNDED_CYCLE"
        );
    }

    #[test]
    fn device_flow_allows_login_prompt_labels_and_credential_references() {
        let mut template = valid_template();
        template["nodes"][1]["config"]["passwordPrompt"] = json!("Password:");
        template["nodes"][1]["config"]["credentialRef"] = json!("serial-login:linux:4:node-b");
        assert_eq!(validate(&task(template)).unwrap(), vec!["linux:4"]);
    }

    #[test]
    fn device_flow_rejects_unreachable_nodes_and_result_outputs() {
        let mut template = valid_template();
        template["edges"] = json!([
            {"id":"ac","source":"a","target":"c"},
            {"id":"bd","source":"b","target":"d"},
            {"id":"cd","source":"c","target":"d","branch":"success"}
        ]);
        assert_eq!(
            validate(&task(template)).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_UNREACHABLE_NODE"
        );

        let mut template = valid_template();
        template["nodes"].as_array_mut().unwrap().push(json!({
            "id":"e", "type":"result.failed", "label":"failed", "x":40, "y":0, "config":{}
        }));
        template["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"de","source":"d","target":"e"}));
        assert_eq!(
            validate(&task(template)).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_TERMINAL_DIRECTION_INVALID"
        );
    }

    #[test]
    fn mcu_scope_uses_the_real_stable_string_id_format() {
        assert!(valid_board_scope("mcu:dshan-ra6m5-v4"));
        assert!(valid_board_scope("mcu:mcu-1785824152732-9e9d18ff"));
        assert!(!valid_board_scope("mcu:../linux:4"));
        assert!(!valid_board_scope("linux:dshan-ra6m5-v4"));
    }

    #[test]
    fn enabled_flow_rejects_nodes_that_are_only_saveable_as_drafts() {
        let mut template = valid_template();
        template["nodes"][1]["type"] = json!("ssh.build");
        template["nodes"][1]["config"] = json!({
            "boardId":"linux:4", "projectId":2, "approvalMode":"required"
        });
        let mut enabled = task(template);
        enabled.enabled = true;
        assert_eq!(
            validate(&enabled).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_EXECUTOR_NOT_AVAILABLE:ssh.build"
        );
    }

    #[test]
    fn executable_nodes_require_complete_inputs_and_raw_serial_requires_approval() {
        let mut template = valid_template();
        template["nodes"][1]["type"] = json!("serial.send");
        template["nodes"][1]["config"] = json!({
            "boardId":"linux:4", "text":"reboot", "appendEnter":true
        });
        assert!(validate(&task(template.clone()))
            .unwrap_err()
            .contains("APPROVAL"));
        template["nodes"][1]["config"]["approvalMode"] = json!("required");
        assert_eq!(validate(&task(template.clone())).unwrap(), vec!["linux:4"]);
        template["nodes"][1]["config"]["text"] = json!("");
        assert!(validate(&task(template))
            .unwrap_err()
            .contains("TEXT_INVALID"));

        let mut template = valid_template();
        template["nodes"][1]["type"] = json!("serial.expect");
        template["nodes"][1]["config"] =
            json!({"boardId":"linux:4", "pattern":"", "timeoutMs":1000});
        assert!(validate(&task(template))
            .unwrap_err()
            .contains("TEXT_INVALID"));
    }
}

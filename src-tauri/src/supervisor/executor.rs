use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::workbench::types::PowerAction;
use crate::workbench::WorkbenchState;

use super::storage;
use super::SupervisorState;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionExecutionResult {
    pub run_id: String,
    pub round_index: u32,
    pub action: String,
    pub action_result: Value,
    pub verification_result: Value,
}

fn linux_device_id(board_id: &str) -> Result<i64, String> {
    let value = board_id
        .strip_prefix("linux:")
        .ok_or_else(|| "SUPERVISOR_ACTION_BOARD_DOMAIN_UNSUPPORTED".to_owned())?;
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| "SUPERVISOR_ACTION_BOARD_ID_INVALID".to_owned())
}

fn bounded_milliseconds(
    parameters: &Value,
    key: &str,
    default: u64,
    max: u64,
) -> Result<u64, String> {
    let value = parameters
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default);
    if value == 0 || value > max {
        return Err(format!("SUPERVISOR_ACTION_PARAMETER_INVALID:{key}"));
    }
    Ok(value)
}

pub fn execute(
    app: &AppHandle,
    supervisor: &Arc<SupervisorState>,
    workbench: &Arc<WorkbenchState>,
    run_id: &str,
    round_index: u32,
    approved: bool,
) -> Result<ActionExecutionResult, String> {
    let (task_id, board_id, _, decision, policy_result) = {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        let (task_id, board_id, policy_id) = storage::run_scope(&db, run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
        let (decision, policy_result, action_result) =
            storage::round_for_execution(&db, run_id, round_index)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SUPERVISOR_ROUND_NOT_FOUND".to_owned())?;
        if action_result != "not_executed" {
            return Err("SUPERVISOR_ACTION_ALREADY_EXECUTED".to_owned());
        }
        (task_id, board_id, policy_id, decision, policy_result)
    };
    if policy_result == "approval_required" && !approved {
        return Err("SUPERVISOR_ACTION_APPROVAL_REQUIRED".to_owned());
    }
    if !matches!(policy_result.as_str(), "allowed" | "approval_required") {
        return Err(format!("SUPERVISOR_ACTION_POLICY_REJECTED:{policy_result}"));
    }
    // Failed flash attempts never authorize an automatic retry. The existing
    // flash UI must restart the exact persisted artifact after operator review.
    if decision.decision == "retry_flash" {
        return Err("SUPERVISOR_FLASH_RETRY_REQUIRES_OPERATOR_WORKFLOW".to_owned());
    }
    if decision.decision == "retry_step" {
        return Err("SUPERVISOR_STEP_RETRY_REQUIRES_OPERATOR_WORKFLOW".to_owned());
    }

    let (action_result, verification_result) = match decision.decision.as_str() {
        "collect_logs" => collect_logs(workbench, &task_id)?,
        "rescan_usb" => scan_bound_device(app, workbench, &board_id, false, 0)?,
        "wait_device" => {
            let timeout_ms =
                bounded_milliseconds(&decision.parameters, "timeout_ms", 30_000, 120_000)?;
            scan_bound_device(app, workbench, &board_id, true, timeout_ms)?
        }
        "power_cycle" => power_action(app, workbench, &board_id, true, &decision.parameters)?,
        "reset_board" => power_action(app, workbench, &board_id, false, &decision.parameters)?,
        other => return Err(format!("SUPERVISOR_ACTION_ADAPTER_NOT_AVAILABLE:{other}")),
    };
    {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        let changed = storage::finish_round_action(
            &db,
            run_id,
            round_index,
            &action_result,
            &verification_result,
            &decision.decision,
        )
        .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("SUPERVISOR_ACTION_RESULT_RACE".to_owned());
        }
    }
    Ok(ActionExecutionResult {
        run_id: run_id.to_owned(),
        round_index,
        action: decision.decision,
        action_result,
        verification_result,
    })
}

fn collect_logs(state: &WorkbenchState, task_id: &str) -> Result<(Value, Value), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let task = db
        .query_row(
            "SELECT status,phase,error,details_json FROM task_history WHERE task_id=?1",
            [task_id],
            |row| Ok(json!({"status": row.get::<_, String>(0)?, "phase": row.get::<_, String>(1)?, "error": row.get::<_, String>(2)?, "details": row.get::<_, String>(3)?})),
        )
        .map_err(|error| format!("SUPERVISOR_TASK_LOGS:{error}"))?;
    let mut statement = db
        .prepare("SELECT step_name,status,error FROM workflow_steps WHERE task_id=?1 ORDER BY id DESC LIMIT 20")
        .map_err(|error| error.to_string())?;
    let steps = statement
        .query_map([task_id], |row| Ok(json!({"step": row.get::<_, String>(0)?, "status": row.get::<_, String>(1)?, "error": row.get::<_, String>(2)?})))
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    Ok((
        json!({"status": "success", "task": task, "steps": steps}),
        json!({"kind": "evidence_collected", "verified": true}),
    ))
}

fn scan_bound_device(
    app: &AppHandle,
    state: &Arc<WorkbenchState>,
    board_id: &str,
    wait: bool,
    timeout_ms: u64,
) -> Result<(Value, Value), String> {
    let device_id = linux_device_id(board_id)?;
    let device = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        crate::workbench::storage::device_by_id(&db, device_id)
            .map_err(|error| error.to_string())?
    };
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin_scoped(
        "AI 监督器",
        "USB 设备探测",
        board_id.to_owned(),
        crate::workbench::flasher::flasher_resource_key(&device.flasher_kind, &device.usb_binding),
    )?;
    let flasher_lock = state.flasher_target_lock(&device.flasher_kind, &device.usb_binding)?;
    let _flasher_guard = flasher_lock
        .try_lock()
        .map_err(|_| format!("FLASHER_BUSY:{}", device.flasher_kind))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let candidates = crate::workbench::flasher::scan_flasher_devices(
            app,
            &device.flasher_kind,
            &device.flasher_path,
        )?;
        let found = candidates
            .iter()
            .any(|candidate| candidate.key == device.usb_binding);
        if found || !wait || Instant::now() >= deadline {
            return Ok((
                json!({"status": "success", "found": found, "candidate_count": candidates.len()}),
                json!({"kind": "bound_usb_present", "verified": found, "binding": device.usb_binding}),
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn power_action(
    app: &AppHandle,
    state: &Arc<WorkbenchState>,
    board_id: &str,
    cycle: bool,
    parameters: &Value,
) -> Result<(Value, Value), String> {
    crate::hardware_safety::ensure_idle(app, "AI 监督器电源操作")?;
    let device_id = linux_device_id(board_id)?;
    let device = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        crate::workbench::storage::device_by_id(&db, device_id)
            .map_err(|error| error.to_string())?
    };
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin(
        "AI 监督器",
        if cycle {
            "电源循环"
        } else {
            "板卡复位"
        },
        board_id.to_owned(),
    )?;
    let lock = state.device_lock(device_id)?;
    let _device_guard = lock
        .try_lock()
        .map_err(|_| "DEVICE_BUSY:board operation in progress".to_owned())?;
    let response = if cycle {
        let off_ms = bounded_milliseconds(parameters, "off_ms", 2_000, 30_000)?;
        crate::workbench::power::execute_and_audit(
            app,
            state,
            Some("ai-supervisor"),
            &device,
            PowerAction::PowerOff,
            0,
            crate::workbench::power::PowerAuditActor::Supervisor,
        )?;
        std::thread::sleep(Duration::from_millis(off_ms));
        crate::workbench::power::execute_and_audit(
            app,
            state,
            Some("ai-supervisor"),
            &device,
            PowerAction::PowerOn,
            0,
            crate::workbench::power::PowerAuditActor::Supervisor,
        )?
    } else {
        let pulse_ms = bounded_milliseconds(parameters, "pulse_ms", 200, 5_000)?;
        crate::workbench::power::execute_and_audit(
            app,
            state,
            Some("ai-supervisor"),
            &device,
            PowerAction::ResetPulse,
            pulse_ms,
            crate::workbench::power::PowerAuditActor::Supervisor,
        )?
    };
    let state_response = crate::workbench::power::execute_and_audit(
        app,
        state,
        Some("ai-supervisor"),
        &device,
        PowerAction::GetState,
        0,
        crate::workbench::power::PowerAuditActor::Supervisor,
    )?;
    Ok((
        json!({"status": "success", "controller_response": response}),
        json!({"kind": "controller_state", "verified": !state_response.trim().is_empty(), "state": state_response}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_scope_must_name_a_domain_and_fixed_id() {
        assert_eq!(linux_device_id("linux:42").unwrap(), 42);
        assert!(linux_device_id("42").is_err());
        assert!(linux_device_id("mcu:42").is_err());
    }

    #[test]
    fn action_parameters_are_bounded() {
        assert_eq!(
            bounded_milliseconds(&json!({}), "off_ms", 2_000, 30_000).unwrap(),
            2_000
        );
        assert!(bounded_milliseconds(&json!({"off_ms": 999999}), "off_ms", 2_000, 30_000).is_err());
    }
}

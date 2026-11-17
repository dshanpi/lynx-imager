use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::serial::SerialState;
use crate::workbench::types::SshCredentials;
use crate::workbench::WorkbenchState;

use super::storage;
use super::types::AutomaticTask;
use super::SupervisorState;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn number(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(Value::as_i64)
}

fn text_value<'a>(value: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(Value::as_str)
}

fn matches_device_event(task: &AutomaticTask, event: &Value, current: i64) -> bool {
    if !task.enabled || task.trigger_type != "device" {
        return false;
    }
    let wanted_event = text_value(&task.trigger, "event", "event").unwrap_or("arrived");
    let actual_event = text_value(event, "event", "event").unwrap_or_default();
    let wanted_vendor = number(&task.trigger, "vendor_id", "vendorId");
    let wanted_product = number(&task.trigger, "product_id", "productId");
    let actual_vendor = number(event, "vendor_id", "vendorId");
    let actual_product = number(event, "product_id", "productId");
    let cooldown = number(&task.trigger, "cooldown_seconds", "cooldownSeconds")
        .unwrap_or(30)
        .clamp(10, 86_400);
    wanted_vendor.is_some()
        && wanted_product.is_some()
        && wanted_event == actual_event
        && wanted_vendor == actual_vendor
        && wanted_product == actual_product
        && task
            .last_run_at
            .is_none_or(|last| current.saturating_sub(last) >= cooldown)
}

pub(super) fn next_schedule(task: &AutomaticTask, current: i64) -> Option<i64> {
    number(&task.trigger, "interval_seconds", "intervalSeconds")
        .filter(|seconds| (10..=31_536_000).contains(seconds))
        .map(|seconds| current.saturating_add(seconds))
}

pub(super) fn template_ids(task: &AutomaticTask) -> Result<(i64, i64), String> {
    let kind = task
        .task_template
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("workflow");
    if kind != "workflow" {
        return Err("SUPERVISOR_AUTOMATIC_TEMPLATE_KIND_UNSUPPORTED".to_owned());
    }
    let project_id = number(&task.task_template, "project_id", "projectId")
        .filter(|id| *id > 0)
        .ok_or_else(|| "SUPERVISOR_AUTOMATIC_PROJECT_REQUIRED".to_owned())?;
    let device_id = number(&task.task_template, "device_id", "deviceId")
        .filter(|id| *id > 0)
        .ok_or_else(|| "SUPERVISOR_AUTOMATIC_DEVICE_REQUIRED".to_owned())?;
    if task.board_id != format!("linux:{device_id}") {
        return Err("SUPERVISOR_AUTOMATIC_BOARD_SCOPE_MISMATCH".to_owned());
    }
    Ok((project_id, device_id))
}

fn verify_terminal_task(task: &AutomaticTask, status: &str, exit_code: Option<i64>) -> bool {
    status == "success"
        && task.success_conditions.iter().all(|condition| {
            match condition.get("kind").and_then(Value::as_str) {
                Some("exit_code") => exit_code == condition.get("equals").and_then(Value::as_i64),
                Some("status") => Some(status) == condition.get("equals").and_then(Value::as_str),
                _ => false,
            }
        })
}

fn monitor_automatic_result(
    app: AppHandle,
    task: AutomaticTask,
    started_task_id: String,
    run_id: String,
) {
    tauri::async_runtime::spawn(async move {
        let supervisor = app.state::<Arc<SupervisorState>>();
        let max_duration = {
            let Ok(db) = supervisor.db.lock() else {
                return;
            };
            task.policy_id
                .and_then(|id| storage::policy(&db, id).ok().flatten())
                .map(|policy| policy.max_duration_seconds)
                .unwrap_or(1_800)
        };
        let deadline = now().saturating_add(i64::from(max_duration));
        loop {
            let workbench = app.state::<Arc<WorkbenchState>>();
            let state = {
                let Ok(db) = workbench.db.lock() else {
                    return;
                };
                db.query_row(
                    "SELECT status,exit_code,error FROM task_history WHERE task_id=?1",
                    [&started_task_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .ok()
            };
            if let Some((status, exit_code, error)) = state {
                if matches!(status.as_str(), "success" | "failed" | "cancelled") {
                    let verified = verify_terminal_task(&task, &status, exit_code);
                    if let Ok(db) = supervisor.db.lock() {
                        let _ = storage::insert_system_audit_for_run(
                            &db,
                            Some(&run_id),
                            &task.board_id,
                            "automatic_verification",
                            if verified { "verified" } else { "failed" },
                            &json!({"automatic_task_id": task.id, "task_id": started_task_id}),
                            &json!({"status": status, "exit_code": exit_code, "verified": verified, "error": error}),
                        );
                        if verified {
                            let _ = storage::set_run_status(
                                &db,
                                &run_id,
                                "success",
                                "自动任务已通过确定性验收",
                            );
                        } else if status == "cancelled" {
                            let _ = storage::set_run_status(
                                &db,
                                &run_id,
                                "cancelled",
                                "自动任务已取消",
                            );
                        } else if status != "failed" || !super::flow::is_device_flow(&task) {
                            let reason = if error.is_empty() {
                                "自动任务未通过确定性验收"
                            } else {
                                &error
                            };
                            let _ = storage::set_run_status(&db, &run_id, "failed", reason);
                        }
                    }
                    let _ = app.emit(
                        "supervisor-automatic-result",
                        json!({"id": task.id, "taskId": started_task_id, "runId": run_id, "verified": verified}),
                    );
                    if !verified && status == "failed" && super::flow::is_device_flow(&task) {
                        if let Some(device_id) = task
                            .board_id
                            .strip_prefix("linux:")
                            .and_then(|id| id.parse::<i64>().ok())
                        {
                            super::schedule_workbench_failure(
                                app.clone(),
                                Some(run_id.clone()),
                                started_task_id.clone(),
                                device_id,
                                "device_flow",
                                if error.is_empty() {
                                    "DEVICE_FLOW_VERIFICATION_FAILED".to_owned()
                                } else {
                                    error.clone()
                                },
                            );
                        }
                    }
                    return;
                }
            }
            if now() >= deadline {
                if let Ok(db) = supervisor.db.lock() {
                    let _ = storage::insert_system_audit_for_run(
                        &db,
                        Some(&run_id),
                        &task.board_id,
                        "automatic_verification",
                        "timeout",
                        &json!({"automatic_task_id": task.id, "task_id": started_task_id}),
                        &json!({"verified": false}),
                    );
                    let _ = storage::set_run_status(
                        &db,
                        &run_id,
                        "failed",
                        "自动任务超过监督时长，未取得终态",
                    );
                }
                let _ = app.emit(
                    "supervisor-automatic-result",
                    json!({"id": task.id, "taskId": started_task_id, "runId": run_id, "verified": false, "timeout": true}),
                );
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn validate_policy_scope(supervisor: &SupervisorState, task: &AutomaticTask) -> Result<(), String> {
    let policy_id = task
        .policy_id
        .ok_or_else(|| "SUPERVISOR_AUTOMATIC_POLICY_REQUIRED".to_owned())?;
    let db = supervisor.db.lock().map_err(|error| error.to_string())?;
    let policy = storage::policy(&db, policy_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "SUPERVISOR_POLICY_NOT_FOUND".to_owned())?;
    if !policy.enabled {
        return Err("SUPERVISOR_POLICY_DISABLED".to_owned());
    }
    let scopes = if super::flow::is_device_flow(task) {
        super::flow::validate(task)?
    } else {
        vec![task.board_id.clone()]
    };
    if scopes
        .iter()
        .any(|scope| !policy.board_scope.iter().any(|board| board == scope))
    {
        return Err("SUPERVISOR_AUTOMATIC_POLICY_SCOPE_MISMATCH".to_owned());
    }
    Ok(())
}

pub fn trigger(
    app: &AppHandle,
    task_id: i64,
    require_enabled: bool,
    operator_approved: bool,
) -> Result<String, String> {
    let supervisor = app.state::<Arc<SupervisorState>>();
    let task = {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        storage::automatic_task(&db, task_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_AUTOMATIC_TASK_NOT_FOUND".to_owned())?
    };
    if require_enabled && !task.enabled {
        return Err("SUPERVISOR_AUTOMATIC_TASK_DISABLED".to_owned());
    }
    validate_policy_scope(&supervisor, &task)?;
    let result = if super::flow::is_device_flow(&task) {
        super::flow::start(app, task.clone(), operator_approved)
    } else {
        if !operator_approved {
            return Err("SUPERVISOR_AUTOMATIC_OPERATOR_APPROVAL_REQUIRED".to_owned());
        }
        let (project_id, device_id) = template_ids(&task)?;
        let workbench = app.state::<Arc<WorkbenchState>>();
        let serial = app.state::<Arc<SerialState>>();
        crate::workbench::workflow::start_workflow(
            project_id,
            device_id,
            SshCredentials::default(),
            app.clone(),
            workbench.inner(),
            serial.inner(),
        )
    };
    let timestamp = now();
    let (result_label, output) = match &result {
        Ok(started_task_id) => ("started", json!({"task_id": started_task_id})),
        Err(error) => ("rejected", json!({"error": error})),
    };
    let automatic_run_id = if let Ok(started_task_id) = &result {
        let policy_id = task
            .policy_id
            .ok_or_else(|| "SUPERVISOR_AUTOMATIC_POLICY_REQUIRED".to_owned())?;
        let run_id = format!("automatic-{started_task_id}");
        let persist_result = (|| {
            let db = supervisor.db.lock().map_err(|error| error.to_string())?;
            storage::create_or_get_run(&db, &run_id, started_task_id, &task.board_id, policy_id)
                .map_err(|error| error.to_string())?;
            storage::run_scope(&db, &run_id)
                .map_err(|error| error.to_string())?
                .filter(|(persisted_task_id, board_id, persisted_policy_id)| {
                    persisted_task_id == started_task_id
                        && board_id == &task.board_id
                        && *persisted_policy_id == policy_id
                })
                .ok_or_else(|| "SUPERVISOR_RUN_NOT_PERSISTED".to_owned())?;
            Ok::<(), String>(())
        })();
        if let Err(error) = persist_result {
            let workbench = app.state::<Arc<WorkbenchState>>();
            if let Ok(tasks) = workbench.cancellations.lock() {
                if let Some(cancellation) = tasks.get(started_task_id) {
                    cancellation.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            return Err(format!("SUPERVISOR_RUN_REGISTRATION_FAILED:{error}"));
        }
        Some(run_id)
    } else {
        None
    };
    {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        storage::insert_system_audit_for_run(
            &db,
            automatic_run_id.as_deref(),
            &task.board_id,
            "automatic_trigger",
            result_label,
            &json!({"automatic_task_id": task_id, "trigger_type": task.trigger_type}),
            &output,
        )
        .map_err(|error| error.to_string())?;
        // A rejected scheduled attempt is still an attempt. Advance its next
        // deadline so a persistent precondition error cannot retry every tick.
        storage::mark_automatic_task_run(&db, task_id, timestamp, next_schedule(&task, timestamp))
            .map_err(|error| error.to_string())?;
    }
    if let Ok(started_task_id) = &result {
        let run_id = automatic_run_id.expect("successful automatic task has a run id");
        let _ = app.emit(
            "supervisor-run-update",
            json!({"runId": run_id, "taskId": started_task_id, "boardId": task.board_id, "status": "running"}),
        );
        monitor_automatic_result(app.clone(), task.clone(), started_task_id.clone(), run_id);
    }
    result
}

pub(crate) fn trigger_webhook(
    app: &AppHandle,
    task_id: i64,
    expected_device_id: i64,
    authorization: Option<&str>,
) -> Result<String, String> {
    let supplied = authorization
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| "SUPERVISOR_WEBHOOK_UNAUTHORIZED".to_owned())?;
    let expected = super::credentials::resolve_webhook(task_id)
        .map_err(|_| "SUPERVISOR_WEBHOOK_UNAUTHORIZED".to_owned())?;
    let supplied_hash = Sha256::digest(supplied.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    if supplied_hash != expected_hash {
        return Err("SUPERVISOR_WEBHOOK_UNAUTHORIZED".to_owned());
    }
    let supervisor = app.state::<Arc<SupervisorState>>();
    let task = {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        storage::automatic_task(&db, task_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_AUTOMATIC_TASK_NOT_FOUND".to_owned())?
    };
    if task.trigger_type != "webhook" || task.board_id != format!("linux:{expected_device_id}") {
        return Err("SUPERVISOR_WEBHOOK_SCOPE_MISMATCH".to_owned());
    }
    let current = now();
    let cooldown = number(&task.trigger, "cooldown_seconds", "cooldownSeconds")
        .unwrap_or(30)
        .clamp(10, 86_400);
    if task
        .last_run_at
        .is_some_and(|last| current.saturating_sub(last) < cooldown)
    {
        return Err("SUPERVISOR_WEBHOOK_COOLDOWN".to_owned());
    }
    trigger(app, task_id, true, false)
}

pub fn start(app: AppHandle) {
    let event_app = app.clone();
    app.listen("usb-hotplug", move |event| {
        let Ok(payload) = serde_json::from_str::<Value>(event.payload()) else {
            return;
        };
        let supervisor = event_app.state::<Arc<SupervisorState>>();
        let tasks = {
            let Ok(db) = supervisor.db.lock() else {
                return;
            };
            storage::list_automatic_tasks(&db).unwrap_or_default()
        };
        let current = now();
        for task in tasks
            .into_iter()
            .filter(|task| matches_device_event(task, &payload, current))
        {
            let Some(id) = task.id else {
                continue;
            };
            match trigger(&event_app, id, true, false) {
                Ok(task_id) => {
                    let _ = event_app.emit(
                        "supervisor-automatic-trigger",
                        json!({"id": id, "taskId": task_id, "status": "started", "source": "device"}),
                    );
                }
                Err(error) => {
                    log::warn!("Device-triggered supervisor task {id} rejected: {error}");
                    let _ = event_app.emit(
                        "supervisor-automatic-trigger",
                        json!({"id": id, "status": "rejected", "source": "device", "error": error}),
                    );
                }
            }
        }
    });

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let supervisor = app.state::<Arc<SupervisorState>>();
            let due = {
                let Ok(db) = supervisor.db.lock() else {
                    continue;
                };
                storage::due_automatic_tasks(&db, now()).unwrap_or_default()
            };
            for task in due {
                let Some(id) = task.id else {
                    continue;
                };
                match trigger(&app, id, true, false) {
                    Ok(task_id) => {
                        let _ = app.emit(
                            "supervisor-automatic-trigger",
                            json!({"id": id, "taskId": task_id, "status": "started"}),
                        );
                    }
                    Err(error) => {
                        // `trigger` may reject before it reaches the workflow
                        // start/audit path (for example after a policy is
                        // disabled). Move the due time forward here as a
                        // final scheduler-level backoff.
                        if let Ok(db) = supervisor.db.lock() {
                            let timestamp = now();
                            let _ = storage::mark_automatic_task_run(
                                &db,
                                id,
                                timestamp,
                                next_schedule(&task, timestamp),
                            );
                        }
                        log::warn!("Automatic supervisor task {id} rejected: {error}");
                        let _ = app.emit(
                            "supervisor-automatic-trigger",
                            json!({"id": id, "status": "rejected", "error": error}),
                        );
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_template_cannot_change_bound_board() {
        let mut task = AutomaticTask {
            id: Some(1),
            name: "test".to_owned(),
            enabled: true,
            trigger_type: "manual_api".to_owned(),
            trigger: json!({}),
            board_id: "linux:4".to_owned(),
            task_template: json!({"projectId": 2, "deviceId": 4}),
            policy_id: None,
            success_conditions: vec![json!({"kind":"exit_code","equals":0})],
            next_run_at: None,
            last_run_at: None,
        };
        assert_eq!(template_ids(&task).unwrap(), (2, 4));
        task.task_template["deviceId"] = json!(5);
        assert!(template_ids(&task).is_err());
    }

    #[test]
    fn webhook_cooldown_is_at_least_ten_seconds() {
        let trigger = json!({"cooldown_seconds": 1});
        let cooldown = number(&trigger, "cooldown_seconds", "cooldownSeconds")
            .unwrap()
            .clamp(10, 86_400);
        assert_eq!(cooldown, 10);
    }

    #[test]
    fn schedules_require_bounded_interval() {
        let mut task = AutomaticTask {
            id: Some(1),
            name: "test".to_owned(),
            enabled: true,
            trigger_type: "schedule".to_owned(),
            trigger: json!({"intervalSeconds": 60}),
            board_id: "linux:1".to_owned(),
            task_template: json!({}),
            policy_id: None,
            success_conditions: vec![],
            next_run_at: None,
            last_run_at: None,
        };
        assert_eq!(next_schedule(&task, 100), Some(160));
        task.trigger = json!({"intervalSeconds": 1});
        assert_eq!(next_schedule(&task, 100), None);
    }

    #[test]
    fn device_trigger_requires_exact_vid_pid_and_event() {
        let task = AutomaticTask {
            id: Some(1),
            name: "fel arrived".to_owned(),
            enabled: true,
            trigger_type: "device".to_owned(),
            trigger: json!({"event":"arrived", "vendorId": 0x1f3a, "productId": 0xefc8}),
            board_id: "linux:1".to_owned(),
            task_template: json!({}),
            policy_id: Some(1),
            success_conditions: vec![],
            next_run_at: None,
            last_run_at: None,
        };
        assert!(matches_device_event(
            &task,
            &json!({"event":"arrived", "vendor_id":0x1f3a, "product_id":0xefc8}),
            100
        ));
        assert!(!matches_device_event(
            &task,
            &json!({"event":"arrived", "vendor_id":0x1f3a, "product_id":1}),
            100
        ));
    }

    #[test]
    fn automatic_success_is_deterministic() {
        let task = AutomaticTask {
            id: Some(1),
            name: "verify".to_owned(),
            enabled: true,
            trigger_type: "manual_api".to_owned(),
            trigger: json!({}),
            board_id: "linux:1".to_owned(),
            task_template: json!({}),
            policy_id: Some(1),
            success_conditions: vec![json!({"kind":"exit_code", "equals":0})],
            next_run_at: None,
            last_run_at: None,
        };
        assert!(verify_terminal_task(&task, "success", Some(0)));
        assert!(!verify_terminal_task(&task, "success", Some(1)));
        assert!(!verify_terminal_task(&task, "failed", Some(0)));
    }
}

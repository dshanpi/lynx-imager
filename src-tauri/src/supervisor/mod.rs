//! AI task supervisor.
//!
//! This domain owns provider configuration, credential references, policies,
//! supervised runs, human intervention requests and immutable audit records.
//! Provider secrets are never stored in SQLite or returned to the webview.

pub mod commands;
mod credentials;
mod engine;
mod executor;
pub(crate) mod flow;
mod flow_runtime;
pub(crate) mod scheduler;
pub(crate) mod storage;
pub mod types;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tauri::{Emitter, Manager};

struct SupervisorTaskLifecycleAdapter;

fn should_start_supervision(completion: &crate::task_lifecycle::WorkbenchTaskCompletion) -> bool {
    completion.status == "failed"
}

impl crate::task_lifecycle::TaskLifecyclePort for SupervisorTaskLifecycleAdapter {
    fn workbench_task_completed(
        &self,
        app: tauri::AppHandle,
        completion: crate::task_lifecycle::WorkbenchTaskCompletion,
    ) {
        if !should_start_supervision(&completion) {
            return;
        }
        schedule_workbench_failure(
            app,
            None,
            completion.task_id,
            completion.device_id,
            &completion.kind,
            completion.error,
        );
    }
}

pub(crate) fn register_task_lifecycle_port(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<crate::task_lifecycle::TaskLifecycleState>()
        .register(Arc::new(SupervisorTaskLifecycleAdapter))
        .map_err(str::to_owned)
}

fn audit_policy_decision(
    app: &tauri::AppHandle,
    context: &types::FailureContext,
    analysis: &types::SupervisorAnalysisResult,
) {
    let error_code = if analysis.execution_allowed {
        None
    } else {
        Some(policy_result_error_code(&analysis.policy_result).to_owned())
    };
    publish_supervisor_audit(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "ai".to_owned(),
            actor_id: "supervisor".to_owned(),
            category: "policy".to_owned(),
            action: "decision_evaluated".to_owned(),
            scope_type: audit_scope_type(&context.board_id).to_owned(),
            scope_id: context.board_id.clone(),
            session_id: Some(analysis.run_id.clone()),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(context.task_id.clone()),
            causation_id: Some(analysis.run_id.clone()),
            result: analysis.policy_result.clone(),
            error_code,
            summary: policy_result_summary(&analysis.policy_result).to_owned(),
            metadata: serde_json::json!({
                "runId": analysis.run_id,
                "roundIndex": analysis.round_index,
                "policyId": context.policy_id,
                "taskKind": context.task_kind,
                "decision": analysis.decision.decision,
                "approvalRequired": analysis.approval_required,
                "executionAllowed": analysis.execution_allowed,
            }),
        },
    );
}

fn audit_human_resolution(
    app: &tauri::AppHandle,
    run_id: &str,
    task_id: &str,
    board_id: &str,
    request_id: i64,
    resolution: &str,
    resulting_status: &str,
) {
    publish_supervisor_audit(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "approval".to_owned(),
            action: "human_request_resolved".to_owned(),
            scope_type: audit_scope_type(board_id).to_owned(),
            scope_id: board_id.to_owned(),
            session_id: Some(run_id.to_owned()),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: Some(run_id.to_owned()),
            result: resolution.to_owned(),
            error_code: None,
            summary: human_resolution_summary(resolution).to_owned(),
            metadata: serde_json::json!({
                "runId": run_id,
                "requestId": request_id,
                "resolution": resolution,
                "resultingStatus": resulting_status,
            }),
        },
    );
}

fn audit_action_approval(
    app: &tauri::AppHandle,
    run_id: &str,
    task_id: &str,
    board_id: &str,
    round_index: u32,
    action: &str,
) {
    publish_supervisor_audit(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "approval".to_owned(),
            action: "supervisor_action_approved".to_owned(),
            scope_type: audit_scope_type(board_id).to_owned(),
            scope_id: board_id.to_owned(),
            session_id: Some(run_id.to_owned()),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: Some(run_id.to_owned()),
            result: "approved".to_owned(),
            error_code: None,
            summary: "操作员批准监督动作并完成执行".to_owned(),
            metadata: serde_json::json!({
                "runId": run_id,
                "roundIndex": round_index,
                "action": action,
            }),
        },
    );
}

fn publish_supervisor_audit(app: &tauri::AppHandle, event: crate::audit::AuditEventDraft) {
    if let Err(error_code) =
        crate::audit::publish(app, event, crate::audit::AuditRequirement::BestEffort)
    {
        log::warn!("Unexpected required Supervisor audit failure: {error_code}");
    }
}

fn policy_result_error_code(result: &str) -> &'static str {
    match result {
        "approval_required" => "SUPERVISOR_APPROVAL_REQUIRED",
        "human_required" => "SUPERVISOR_HUMAN_REQUIRED",
        "rejected_budget" => "SUPERVISOR_POLICY_BUDGET_REJECTED",
        "rejected_no_progress" => "SUPERVISOR_POLICY_NO_PROGRESS",
        "verification_required" => "SUPERVISOR_VERIFICATION_REQUIRED",
        "final_failed" => "SUPERVISOR_FINAL_FAILED",
        _ => "SUPERVISOR_POLICY_NOT_ALLOWED",
    }
}

fn policy_result_summary(result: &str) -> &'static str {
    match result {
        "allowed" => "监督策略允许执行枚举动作",
        "approval_required" => "监督策略要求人工批准",
        "human_required" => "监督策略转入人工介入",
        "rejected_budget" => "监督策略因预算拒绝动作",
        "rejected_no_progress" => "监督策略因重复无进展拒绝动作",
        "verification_required" => "成功候选等待确定性验收",
        "final_failed" => "监督策略确认失败终态",
        _ => "监督策略完成判断",
    }
}

fn human_resolution_summary(resolution: &str) -> &'static str {
    match resolution {
        "continue" => "操作员批准继续监督",
        "manual" => "操作员转为手动处理",
        "terminate" => "操作员终止监督",
        _ => "操作员完成人工处置",
    }
}

fn audit_scope_type(board_id: &str) -> &'static str {
    if board_id.starts_with("mcu:") {
        "mcu_board"
    } else if board_id.starts_with("linux:") {
        "linux_board"
    } else {
        "board"
    }
}

pub struct SupervisorState {
    pub(crate) db: Mutex<Connection>,
    pub(crate) http: reqwest::Client,
}

pub(crate) fn redact_learning_context(value: &str) -> String {
    engine::redact_text(value)
}

impl SupervisorState {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        storage::migrate(&connection).map_err(|error| error.to_string())?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("DshanPI-LYNX/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            db: Mutex::new(connection),
            http,
        })
    }
}

pub fn reconcile_interrupted_runs(
    supervisor: &SupervisorState,
    workbench: &crate::workbench::WorkbenchState,
) -> Result<usize, String> {
    let supervisor_db = supervisor.db.lock().map_err(|error| error.to_string())?;
    let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
    storage::reconcile_interrupted_runs(&supervisor_db, &workbench_db)
        .map_err(|error| error.to_string())
}

pub(crate) fn channel_automatic_tasks(
    app: &tauri::AppHandle,
    board_id: &str,
) -> Result<Vec<types::AutomaticTask>, String> {
    let supervisor = app.state::<Arc<SupervisorState>>();
    let db = supervisor.db.lock().map_err(|error| error.to_string())?;
    storage::list_automatic_tasks(&db)
        .map(|tasks| {
            tasks
                .into_iter()
                .filter(|task| task.board_id == board_id && flow::is_device_flow(task))
                .collect()
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn trigger_channel_automatic_task(
    app: &tauri::AppHandle,
    board_id: &str,
    task_id: i64,
    operator_approved: bool,
) -> Result<String, String> {
    let supervisor = app.state::<Arc<SupervisorState>>();
    let task = {
        let db = supervisor.db.lock().map_err(|error| error.to_string())?;
        storage::automatic_task(&db, task_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_AUTOMATIC_TASK_NOT_FOUND".to_owned())?
    };
    if task.board_id != board_id {
        return Err("SUPERVISOR_AUTOMATIC_TASK_CHANNEL_SCOPE_MISMATCH".to_owned());
    }
    if !flow::is_device_flow(&task) {
        return Err("SUPERVISOR_MCP_DEVICE_FLOW_REQUIRED".to_owned());
    }
    scheduler::trigger(app, task_id, false, operator_approved)
}

pub(crate) fn schedule_workbench_failure(
    app: tauri::AppHandle,
    existing_run_id: Option<String>,
    task_id: String,
    device_id: i64,
    task_kind: &str,
    error: String,
) {
    let supervisor = app.state::<Arc<SupervisorState>>().inner().clone();
    let workbench = app
        .state::<Arc<crate::workbench::WorkbenchState>>()
        .inner()
        .clone();
    let board_id = format!("linux:{device_id}");
    let policy = {
        let Ok(db) = supervisor.db.lock() else {
            return;
        };
        let selected = if let Some(run_id) = existing_run_id.as_deref() {
            storage::run_scope(&db, run_id)
                .map_err(|error| error.to_string())
                .and_then(|scope| {
                    let (_, scoped_board, policy_id) =
                        scope.ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
                    if scoped_board != board_id {
                        return Err("SUPERVISOR_RUN_SCOPE_MISMATCH".to_owned());
                    }
                    storage::policy(&db, policy_id).map_err(|error| error.to_string())
                })
        } else {
            storage::policy_for_board(&db, &board_id)
        };
        match selected {
            Ok(Some(policy)) => policy,
            Ok(None) => return,
            Err(error) => {
                log::error!("AI supervisor policy selection rejected for {board_id}: {error}");
                let _ = app.emit(
                    "supervisor-run-update",
                    serde_json::json!({
                        "taskId": task_id,
                        "boardId": board_id,
                        "status": "configuration_error",
                        "error": error,
                    }),
                );
                return;
            }
        }
    };
    let phase = {
        let Ok(db) = workbench.db.lock() else {
            return;
        };
        db.query_row(
            "SELECT phase FROM task_history WHERE task_id=?1",
            [&task_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "unknown".to_owned())
    };
    let context = types::FailureContext {
        run_id: existing_run_id.clone(),
        task_id: task_id.clone(),
        board_id: board_id.clone(),
        policy_id: policy.id.unwrap_or_default(),
        task_kind: task_kind.to_owned(),
        step_name: phase,
        failure_type: error.split(':').next().unwrap_or("TASK_FAILED").to_owned(),
        error_code: error.split_once(':').map(|(code, _)| code.to_owned()),
        key_logs: vec![error],
        device_state: serde_json::json!({"domain": "linux", "device_id": device_id}),
        firmware_hash: None,
    };
    tauri::async_runtime::spawn(async move {
        let mut context = context;
        loop {
            let analysis = match engine::analyze(Some(&app), &supervisor, context.clone()).await {
                Ok(analysis) => analysis,
                Err(error) => {
                    log::warn!("AI supervisor analysis stopped for task={task_id}: {error}");
                    if let Some(run_id) = &existing_run_id {
                        if let Ok(db) = supervisor.db.lock() {
                            let _ = storage::set_run_status(&db, run_id, "failed", &error);
                        }
                    }
                    let _ = app.emit(
                        "supervisor-run-update",
                        serde_json::json!({
                            "taskId": task_id,
                            "boardId": board_id,
                            "status": "analysis_stopped",
                            "error": error,
                        }),
                    );
                    return;
                }
            };
            let _ = app.emit("supervisor-run-update", &analysis);

            if analysis.policy_result == "verification_required" {
                let task_succeeded = workbench
                    .db
                    .lock()
                    .ok()
                    .and_then(|db| {
                        db.query_row(
                            "SELECT status FROM task_history WHERE task_id=?1",
                            [&task_id],
                            |row| row.get::<_, String>(0),
                        )
                        .ok()
                    })
                    .is_some_and(|status| status == "success");
                if task_succeeded {
                    if let Ok(db) = supervisor.db.lock() {
                        let _ = storage::set_run_status(
                            &db,
                            &analysis.run_id,
                            "success",
                            "原任务确定性状态已验收为 success",
                        );
                    }
                    return;
                }
                context.run_id = Some(analysis.run_id);
                context.step_name = "deterministic_verification".to_owned();
                context.failure_type = "VERIFICATION_FAILED".to_owned();
                context.error_code = Some("ORIGINAL_TASK_NOT_SUCCESS".to_owned());
                context.key_logs = vec![
                    "确定性验收失败：原任务状态不是 success，不能接受 AI 的完成判断。".to_owned(),
                ];
                continue;
            }
            if !analysis.execution_allowed {
                return;
            }

            let app_for_action = app.clone();
            let supervisor_for_action = supervisor.clone();
            let workbench_for_action = workbench.clone();
            let run_id = analysis.run_id.clone();
            let round_index = analysis.round_index;
            let execution = tauri::async_runtime::spawn_blocking(move || {
                executor::execute(
                    &app_for_action,
                    &supervisor_for_action,
                    &workbench_for_action,
                    &run_id,
                    round_index,
                    false,
                )
            })
            .await;
            match execution {
                Ok(Ok(result)) => {
                    let action_evidence = serde_json::to_string(&serde_json::json!({
                        "action": &result.action,
                        "action_result": &result.action_result,
                        "verification_result": &result.verification_result,
                    }))
                    .unwrap_or_else(|_| "恢复动作已完成".to_owned());
                    let _ = app.emit("supervisor-action-result", &result);
                    if result
                        .verification_result
                        .get("verified")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        if let Ok(db) = supervisor.db.lock() {
                            let _ = storage::create_human_request(
                                &db,
                                &analysis.run_id,
                                "恢复动作已通过确定性验收，但原任务仍处于失败状态",
                                "请检查监督证据后决定是否手动重试原任务；LYNX 不会自动重试失败的硬件步骤。",
                                std::slice::from_ref(&action_evidence),
                            );
                        }
                        let _ = app.emit(
                            "supervisor-run-update",
                            serde_json::json!({
                                "runId": analysis.run_id,
                                "taskId": task_id,
                                "boardId": board_id,
                                "status": "waiting_human",
                                "reason": "recovery_verified_original_task_failed",
                            }),
                        );
                        return;
                    }
                    context.run_id = Some(analysis.run_id);
                    context.step_name = "recovery_verification".to_owned();
                    context.failure_type = "RECOVERY_RESULT".to_owned();
                    context.error_code = None;
                    context.key_logs = vec![action_evidence];
                }
                Ok(Err(error)) => {
                    log::warn!("AI supervisor action failed for task={task_id}: {error}");
                    if let Ok(db) = supervisor.db.lock() {
                        let _ = storage::fail_round_action(
                            &db,
                            &analysis.run_id,
                            analysis.round_index,
                            &analysis.decision.decision,
                            &error,
                        );
                    }
                    context.run_id = Some(analysis.run_id);
                    context.step_name = "recovery_action".to_owned();
                    context.failure_type = "ACTION_FAILED".to_owned();
                    context.error_code = Some(error.clone());
                    context.key_logs = vec![format!(
                        "恢复动作 {} 执行失败：{}",
                        analysis.decision.decision, error
                    )];
                }
                Err(error) => {
                    log::warn!("AI supervisor action worker failed: {error}");
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod lifecycle_adapter_tests {
    use super::*;

    fn completion(status: &str) -> crate::task_lifecycle::WorkbenchTaskCompletion {
        crate::task_lifecycle::WorkbenchTaskCompletion {
            task_id: "task-1".to_owned(),
            device_id: 4,
            kind: "workflow".to_owned(),
            status: status.to_owned(),
            error: String::new(),
        }
    }

    #[test]
    fn only_failed_workbench_facts_start_supervision() {
        assert!(should_start_supervision(&completion("failed")));
        assert!(!should_start_supervision(&completion("success")));
        assert!(!should_start_supervision(&completion("cancelled")));
    }

    #[test]
    fn audit_classification_preserves_policy_and_board_semantics() {
        assert_eq!(audit_scope_type("linux:4"), "linux_board");
        assert_eq!(audit_scope_type("mcu:board-1"), "mcu_board");
        assert_eq!(
            policy_result_error_code("approval_required"),
            "SUPERVISOR_APPROVAL_REQUIRED"
        );
        assert_eq!(human_resolution_summary("terminate"), "操作员终止监督");
    }
}

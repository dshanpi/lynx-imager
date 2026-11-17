use std::sync::{Arc, RwLock};

use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchTaskCompletion {
    pub task_id: String,
    pub device_id: i64,
    pub kind: String,
    pub status: String,
    pub error: String,
}

pub trait TaskLifecyclePort: Send + Sync {
    fn workbench_task_completed(&self, app: tauri::AppHandle, completion: WorkbenchTaskCompletion);
}

#[derive(Default)]
pub struct TaskLifecycleState {
    port: RwLock<Option<Arc<dyn TaskLifecyclePort>>>,
}

impl TaskLifecycleState {
    pub fn register(&self, port: Arc<dyn TaskLifecyclePort>) -> Result<(), &'static str> {
        let mut current = self
            .port
            .write()
            .map_err(|_| "TASK_LIFECYCLE_PORT_UNAVAILABLE")?;
        if current.is_some() {
            return Err("TASK_LIFECYCLE_PORT_ALREADY_REGISTERED");
        }
        *current = Some(port);
        Ok(())
    }

    fn subscriber(&self) -> Option<Arc<dyn TaskLifecyclePort>> {
        self.port.read().ok().and_then(|port| port.clone())
    }
}

pub fn publish_workbench_completion(app: &tauri::AppHandle, completion: WorkbenchTaskCompletion) {
    let result = completion.status.clone();
    let error_code = match completion.status.as_str() {
        "failed" => Some("TASK_EXECUTION_FAILED".to_owned()),
        "cancelled" => Some("TASK_EXECUTION_CANCELLED".to_owned()),
        _ => None,
    };
    let summary = match completion.status.as_str() {
        "success" => "Workbench 任务执行成功",
        "failed" => "Workbench 任务执行失败",
        "cancelled" => "Workbench 任务已取消",
        _ => "Workbench 任务已结束",
    };
    let audit_outcome = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "service".to_owned(),
            actor_id: "workbench".to_owned(),
            category: "task".to_owned(),
            action: "completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", completion.device_id),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(completion.task_id.clone()),
            causation_id: None,
            result,
            error_code,
            summary: summary.to_owned(),
            metadata: serde_json::json!({
                "taskId": completion.task_id.clone(),
                "kind": completion.kind.clone(),
            }),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
    if let Err(error_code) = audit_outcome {
        log::warn!("Unexpected required audit failure on task completion: {error_code}");
    }

    let state = app.state::<TaskLifecycleState>();
    if let Some(port) = state.subscriber() {
        port.workbench_task_completed(app.clone(), completion);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchTaskState {
    Running,
    Success,
    Failed,
    Cancelled,
}

impl WorkbenchTaskState {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

pub fn validate_workbench_transition(current: &str, next: &str) -> Result<(), &'static str> {
    let current = WorkbenchTaskState::parse(current).ok_or("TASK_STATE_CURRENT_INVALID")?;
    let next = WorkbenchTaskState::parse(next).ok_or("TASK_STATE_TARGET_INVALID")?;
    if current == next {
        return Ok(());
    }
    if current.is_terminal()
        || !matches!(current, WorkbenchTaskState::Running)
        || !next.is_terminal()
    {
        return Err("TASK_STATE_TRANSITION_INVALID");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorRunState {
    Running,
    Paused,
    WaitingApproval,
    WaitingHuman,
    Manual,
    Terminated,
    Success,
    Failed,
    Cancelled,
}

impl SupervisorRunState {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "waiting_approval" => Some(Self::WaitingApproval),
            "waiting_human" => Some(Self::WaitingHuman),
            "manual" => Some(Self::Manual),
            "terminated" => Some(Self::Terminated),
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Manual | Self::Terminated | Self::Success | Self::Failed | Self::Cancelled
        )
    }
}

pub fn validate_supervisor_transition(current: &str, next: &str) -> Result<(), &'static str> {
    use SupervisorRunState::*;
    let current = SupervisorRunState::parse(current).ok_or("SUPERVISOR_STATE_CURRENT_INVALID")?;
    let next = SupervisorRunState::parse(next).ok_or("SUPERVISOR_STATE_TARGET_INVALID")?;
    if current == next {
        return Ok(());
    }
    if current.is_terminal() {
        return Err("SUPERVISOR_STATE_TERMINAL_IMMUTABLE");
    }
    let allowed = match current {
        Running => matches!(
            next,
            Paused
                | WaitingApproval
                | WaitingHuman
                | Manual
                | Terminated
                | Success
                | Failed
                | Cancelled
        ),
        Paused => matches!(
            next,
            Running | WaitingHuman | Manual | Terminated | Failed | Cancelled
        ),
        WaitingApproval => matches!(
            next,
            Running | WaitingHuman | Manual | Terminated | Failed | Cancelled
        ),
        WaitingHuman => matches!(next, Running | Manual | Terminated | Failed | Cancelled),
        Manual | Terminated | Success | Failed | Cancelled => false,
    };
    if allowed {
        Ok(())
    } else {
        Err("SUPERVISOR_STATE_TRANSITION_INVALID")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingPort(std::sync::Mutex<Vec<WorkbenchTaskCompletion>>);

    impl TaskLifecyclePort for RecordingPort {
        fn workbench_task_completed(
            &self,
            _app: tauri::AppHandle,
            completion: WorkbenchTaskCompletion,
        ) {
            self.0.lock().unwrap().push(completion);
        }
    }

    #[test]
    fn workbench_execution_fact_only_moves_from_running_to_terminal() {
        for terminal in ["success", "failed", "cancelled"] {
            assert!(validate_workbench_transition("running", terminal).is_ok());
            assert!(validate_workbench_transition(terminal, terminal).is_ok());
            assert_eq!(
                validate_workbench_transition(terminal, "running"),
                Err("TASK_STATE_TRANSITION_INVALID")
            );
        }
    }

    #[test]
    fn supervisor_control_states_follow_declared_matrix_and_terminals_are_immutable() {
        assert!(validate_supervisor_transition("running", "paused").is_ok());
        assert!(validate_supervisor_transition("paused", "running").is_ok());
        assert!(validate_supervisor_transition("waiting_approval", "waiting_human").is_ok());
        assert!(validate_supervisor_transition("waiting_human", "manual").is_ok());
        assert_eq!(
            validate_supervisor_transition("success", "failed"),
            Err("SUPERVISOR_STATE_TERMINAL_IMMUTABLE")
        );
    }

    #[test]
    fn lifecycle_port_is_registered_once() {
        let state = TaskLifecycleState::default();
        let port = Arc::new(RecordingPort(std::sync::Mutex::new(Vec::new())));
        assert!(state.register(port.clone()).is_ok());
        assert_eq!(
            state.register(port),
            Err("TASK_LIFECYCLE_PORT_ALREADY_REGISTERED")
        );
    }
}

use std::sync::{Arc, RwLock};

use serde_json::Value;
use tauri::Manager;

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEventDraft {
    pub actor_type: String,
    pub actor_id: String,
    pub category: String,
    pub action: String,
    pub scope_type: String,
    pub scope_id: String,
    pub session_id: Option<String>,
    pub session_kind: Option<String>,
    pub session_title: Option<String>,
    pub end_session: bool,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub result: String,
    pub error_code: Option<String>,
    pub summary: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditRequirement {
    BestEffort,
    #[allow(dead_code)] // Activated only as individually approved high-risk actions migrate.
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditWriteOutcome {
    Recorded { event_id: String },
    Degraded { error_code: String },
}

pub trait AuditSink: Send + Sync {
    fn append(&self, event: AuditEventDraft) -> Result<String, String>;
}

#[derive(Default)]
pub struct AuditSinkState {
    sink: RwLock<Option<Arc<dyn AuditSink>>>,
}

impl AuditSinkState {
    pub fn register(&self, sink: Arc<dyn AuditSink>) -> Result<(), &'static str> {
        let mut current = self.sink.write().map_err(|_| "AUDIT_SINK_UNAVAILABLE")?;
        if current.is_some() {
            return Err("AUDIT_SINK_ALREADY_REGISTERED");
        }
        *current = Some(sink);
        Ok(())
    }

    fn write(
        &self,
        event: AuditEventDraft,
        requirement: AuditRequirement,
    ) -> Result<AuditWriteOutcome, String> {
        let sink = self.sink.read().ok().and_then(|sink| sink.clone());
        let Some(sink) = sink else {
            return degrade_or_reject(requirement, "AUDIT_SINK_NOT_REGISTERED");
        };
        match sink.append(event) {
            Ok(event_id) => Ok(AuditWriteOutcome::Recorded { event_id }),
            Err(error_code) => degrade_or_reject(requirement, &normalize_error_code(&error_code)),
        }
    }
}

fn normalize_error_code(error: &str) -> String {
    let trimmed = error.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= 96
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        trimmed.to_owned()
    } else {
        "AUDIT_WRITE_FAILED".to_owned()
    }
}

pub(crate) fn machine_error_code(error: &str, fallback: &str) -> String {
    let candidate = error.split(':').next().unwrap_or_default();
    let normalized = normalize_error_code(candidate);
    if normalized == "AUDIT_WRITE_FAILED" {
        fallback.to_owned()
    } else {
        normalized
    }
}

fn degrade_or_reject(
    requirement: AuditRequirement,
    error_code: &str,
) -> Result<AuditWriteOutcome, String> {
    match requirement {
        AuditRequirement::BestEffort => Ok(AuditWriteOutcome::Degraded {
            error_code: error_code.to_owned(),
        }),
        AuditRequirement::Required => Err(error_code.to_owned()),
    }
}

pub fn publish(
    app: &tauri::AppHandle,
    event: AuditEventDraft,
    requirement: AuditRequirement,
) -> Result<AuditWriteOutcome, String> {
    let diagnostic_event = event.clone();
    let outcome = app.state::<AuditSinkState>().write(event, requirement)?;
    crate::diagnostics::capture_audit_failure(app, &diagnostic_event);
    if let AuditWriteOutcome::Degraded { error_code } = &outcome {
        log::warn!("Audit event degraded: {error_code}");
    }
    Ok(outcome)
}

/// Use this entry point before a future high-risk operation when its approved
/// requirement states that an audit gap must block execution.
#[allow(dead_code)]
pub fn publish_required(
    app: &tauri::AppHandle,
    event: AuditEventDraft,
) -> Result<AuditWriteOutcome, String> {
    publish(app, event, AuditRequirement::Required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingSink(Mutex<Vec<AuditEventDraft>>);

    impl AuditSink for RecordingSink {
        fn append(&self, event: AuditEventDraft) -> Result<String, String> {
            self.0.lock().unwrap().push(event);
            Ok("evt-recorded".to_owned())
        }
    }

    struct FailingSink(&'static str);

    impl AuditSink for FailingSink {
        fn append(&self, _event: AuditEventDraft) -> Result<String, String> {
            Err(self.0.to_owned())
        }
    }

    fn event() -> AuditEventDraft {
        AuditEventDraft {
            actor_type: "service".to_owned(),
            actor_id: "workbench".to_owned(),
            category: "task".to_owned(),
            action: "completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: "linux:4".to_owned(),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some("task-1".to_owned()),
            causation_id: None,
            result: "success".to_owned(),
            error_code: None,
            summary: "任务执行成功".to_owned(),
            metadata: serde_json::json!({"kind": "workflow"}),
        }
    }

    #[test]
    fn sink_is_registered_once_and_records_structured_event() {
        let state = AuditSinkState::default();
        let sink = Arc::new(RecordingSink(Mutex::new(Vec::new())));
        assert!(state.register(sink.clone()).is_ok());
        assert_eq!(
            state.register(sink.clone()),
            Err("AUDIT_SINK_ALREADY_REGISTERED")
        );
        assert_eq!(
            state.write(event(), AuditRequirement::Required).unwrap(),
            AuditWriteOutcome::Recorded {
                event_id: "evt-recorded".to_owned()
            }
        );
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn failure_policy_degrades_best_effort_and_rejects_required_events() {
        let empty = AuditSinkState::default();
        assert_eq!(
            empty.write(event(), AuditRequirement::BestEffort).unwrap(),
            AuditWriteOutcome::Degraded {
                error_code: "AUDIT_SINK_NOT_REGISTERED".to_owned()
            }
        );
        assert_eq!(
            empty.write(event(), AuditRequirement::Required),
            Err("AUDIT_SINK_NOT_REGISTERED".to_owned())
        );

        let failing = AuditSinkState::default();
        failing
            .register(Arc::new(FailingSink("database path leaked")))
            .unwrap();
        assert_eq!(
            failing
                .write(event(), AuditRequirement::BestEffort)
                .unwrap(),
            AuditWriteOutcome::Degraded {
                error_code: "AUDIT_WRITE_FAILED".to_owned()
            }
        );
        assert_eq!(
            failing.write(event(), AuditRequirement::Required),
            Err("AUDIT_WRITE_FAILED".to_owned())
        );
    }
}

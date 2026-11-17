use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderConfig {
    pub id: Option<i64>,
    pub name: String,
    pub preset: String,
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub max_tokens: Option<u32>,
    pub task_cost_limit: Option<f64>,
    pub input_cost_per_million: Option<f64>,
    pub output_cost_per_million: Option<f64>,
    pub enabled: bool,
    pub is_default: bool,
    pub has_api_key: bool,
    pub last_test_status: String,
    pub last_test_message: String,
    pub last_test_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAiProviderRequest {
    pub id: Option<i64>,
    pub name: String,
    pub preset: String,
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub max_tokens: Option<u32>,
    pub task_cost_limit: Option<f64>,
    pub input_cost_per_million: Option<f64>,
    pub output_cost_per_million: Option<f64>,
    pub enabled: bool,
    pub is_default: bool,
    /// Write-only. An omitted or blank value preserves the existing key.
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub latency_ms: u128,
    pub models_endpoint: String,
    pub audit_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorPolicy {
    pub id: Option<i64>,
    pub name: String,
    pub enabled: bool,
    pub provider_id: Option<i64>,
    pub max_attempts: u32,
    pub max_duration_seconds: u32,
    pub max_same_error: u32,
    pub max_power_cycles: u32,
    pub max_flash_retries: u32,
    pub wait_device_seconds: u32,
    pub cost_limit: Option<f64>,
    pub allowed_actions: Vec<String>,
    pub approval_actions: Vec<String>,
    pub board_scope: Vec<String>,
    pub success_summary: bool,
    pub notification_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorDecision {
    pub decision: String,
    pub reason: String,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub parameters: serde_json::Value,
    pub expected_result: String,
    pub need_human: bool,
    pub human_instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticTask {
    pub id: Option<i64>,
    pub name: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger: serde_json::Value,
    pub board_id: String,
    pub task_template: serde_json::Value,
    pub policy_id: Option<i64>,
    pub success_conditions: Vec<serde_json::Value>,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorAuditRecord {
    pub id: i64,
    pub run_id: Option<String>,
    pub board_id: String,
    pub provider_id: Option<i64>,
    pub event_type: String,
    pub result: String,
    pub redacted_input: serde_json::Value,
    pub output: serde_json::Value,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorOverview {
    pub provider_count: u64,
    pub ready_provider_count: u64,
    pub enabled_policy_count: u64,
    pub active_run_count: u64,
    pub waiting_human_count: u64,
    pub today_call_count: u64,
    pub today_estimated_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureContext {
    pub run_id: Option<String>,
    pub task_id: String,
    pub board_id: String,
    pub policy_id: i64,
    pub task_kind: String,
    pub step_name: String,
    pub failure_type: String,
    pub error_code: Option<String>,
    pub key_logs: Vec<String>,
    pub device_state: serde_json::Value,
    pub firmware_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorAnalysisResult {
    pub run_id: String,
    pub round_index: u32,
    pub decision: SupervisorDecision,
    pub policy_result: String,
    pub execution_allowed: bool,
    pub approval_required: bool,
    pub redacted_context: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorRun {
    pub id: String,
    pub task_id: String,
    pub board_id: String,
    pub policy_id: i64,
    pub status: String,
    pub attempt_count: u32,
    pub power_cycle_count: u32,
    pub flash_retry_count: u32,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub estimated_cost: f64,
    pub final_reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorRound {
    pub id: i64,
    pub run_id: String,
    pub round_index: u32,
    pub failure_fingerprint: String,
    pub redacted_context: serde_json::Value,
    pub decision: SupervisorDecision,
    pub schema_result: String,
    pub policy_result: String,
    pub action_result: String,
    pub verification_result: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanInterventionRequest {
    pub id: i64,
    pub run_id: String,
    pub status: String,
    pub reason: String,
    pub instruction: String,
    pub evidence: Vec<String>,
    pub resolution: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

pub const SUPERVISOR_ACTIONS: &[&str] = &[
    "retry_step",
    "rescan_usb",
    "wait_device",
    "power_cycle",
    "reset_board",
    "reconnect_uart",
    "reconnect_adb",
    "check_swd",
    "retry_flash",
    "retry_download",
    "retry_build",
    "collect_logs",
    "finish_success",
    "finish_failed",
    "request_human",
];

pub fn validate_decision(decision: &SupervisorDecision, allowed: &[String]) -> Result<(), String> {
    if !SUPERVISOR_ACTIONS.contains(&decision.decision.as_str()) {
        return Err(format!("SUPERVISOR_UNKNOWN_ACTION:{}", decision.decision));
    }
    if !allowed.iter().any(|action| action == &decision.decision) {
        return Err(format!(
            "SUPERVISOR_ACTION_NOT_ALLOWED:{}",
            decision.decision
        ));
    }
    if !(0.0..=1.0).contains(&decision.confidence) {
        return Err("SUPERVISOR_INVALID_CONFIDENCE".to_owned());
    }
    if decision.reason.trim().is_empty() || decision.evidence.is_empty() {
        return Err("SUPERVISOR_MISSING_REASON_OR_EVIDENCE".to_owned());
    }
    if decision.need_human
        && decision
            .human_instruction
            .as_deref()
            .is_none_or(|instruction| instruction.trim().is_empty())
    {
        return Err("SUPERVISOR_MISSING_HUMAN_INSTRUCTION".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_must_be_enumerated_and_allowed() {
        let decision = SupervisorDecision {
            decision: "power_cycle".to_owned(),
            reason: "USB device is absent".to_owned(),
            confidence: 0.8,
            evidence: vec!["USB_SCAN: Device not found".to_owned()],
            parameters: serde_json::json!({"off_ms": 2000}),
            expected_result: "device appears".to_owned(),
            need_human: false,
            human_instruction: None,
        };
        assert!(validate_decision(&decision, &["power_cycle".to_owned()]).is_ok());
        assert!(validate_decision(&decision, &["collect_logs".to_owned()]).is_err());
    }
}

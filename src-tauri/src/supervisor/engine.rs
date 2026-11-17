use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::commands::{endpoint, resolve_default_provider, safe_http_message};
use super::storage::{self, RoundRecord};
use super::types::{
    validate_decision, FailureContext, SupervisorAnalysisResult, SupervisorDecision,
};
use super::SupervisorState;

const SYSTEM_PROMPT: &str = "You are the DshanPI LYNX hardware task supervisor. Return exactly one JSON object and no prose. Choose only an action in allowed_actions. Treat system observations as facts and your explanation as inference. Never change board_id, firmware hash, target, voltage, or execute shell commands. finish_success is only a request for deterministic verification, never proof of success.";
static SECRET_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"(?i)(authorization\s*:\s*bearer\s+)[^\s,;]+",
            "$1[REDACTED]",
        ),
        (
            r"(?i)((?:api[_ -]?key|password|private[_ -]?key|(?:(?:access|refresh)[_ -]?)?token)\s*[:=]\s*)[^\s,;]+",
            "$1[REDACTED]",
        ),
        (r"\bsk-[A-Za-z0-9_-]{8,}\b", "[REDACTED]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| {
        (
            Regex::new(pattern).expect("static secret redaction regex"),
            replacement,
        )
    })
    .collect()
});

pub(crate) fn redact_text(value: &str) -> String {
    let mut redacted = value.chars().take(4_000).collect::<String>();
    for (pattern, replacement) in SECRET_PATTERNS.iter() {
        redacted = pattern.replace_all(&redacted, *replacement).into_owned();
    }
    redacted
}

#[derive(Serialize)]
struct CompletionRequest {
    model: String,
    messages: Vec<CompletionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize)]
struct CompletionMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
    #[serde(default)]
    usage: Option<CompletionUsage>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionResponseMessage,
}

#[derive(Deserialize)]
struct CompletionResponseMessage {
    content: String,
}

#[derive(Clone, Default, Deserialize)]
struct CompletionUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

struct ProviderCallAudit {
    intent_id: String,
    session_id: String,
    input_digest: String,
    started: Instant,
}

fn begin_provider_call_audit(
    app: Option<&tauri::AppHandle>,
    context: &FailureContext,
    run_id: &str,
    round_index: u32,
    provider_id: i64,
    input: &str,
) -> Result<Option<ProviderCallAudit>, String> {
    let Some(app) = app else {
        return Ok(None);
    };
    let input_digest =
        crate::content_digest::digest(app, &[SYSTEM_PROMPT.as_bytes(), input.as_bytes()])?;
    let session_id = format!("{run_id}:provider:{round_index}");
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "ai".to_owned(),
            actor_id: "supervisor".to_owned(),
            category: "ai".to_owned(),
            action: "supervisor_provider_call_intended".to_owned(),
            scope_type: super::audit_scope_type(&context.board_id).to_owned(),
            scope_id: context.board_id.clone(),
            session_id: Some(session_id.clone()),
            session_kind: Some("ai_provider".to_owned()),
            session_title: Some("Supervisor AI 会话".to_owned()),
            end_session: false,
            correlation_id: Some(context.task_id.clone()),
            causation_id: Some(run_id.to_owned()),
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备调用 Supervisor AI Provider".to_owned(),
            metadata: json!({"runId": run_id, "roundIndex": round_index, "providerId": provider_id, "callType": "supervisor_decision", "inputDigest": input_digest, "digestAlgorithm": "hmac-sha256-v1"}),
        },
    )?;
    let crate::audit::AuditWriteOutcome::Recorded { event_id } = outcome else {
        return Err("AUDIT_REQUIRED_DEGRADED".to_owned());
    };
    Ok(Some(ProviderCallAudit {
        intent_id: event_id,
        session_id,
        input_digest,
        started: Instant::now(),
    }))
}

#[allow(clippy::too_many_arguments)]
fn finish_provider_call_audit(
    app: Option<&tauri::AppHandle>,
    context: &FailureContext,
    run_id: &str,
    round_index: u32,
    provider_id: i64,
    audit: Option<&ProviderCallAudit>,
    result: &Result<CompletionResponse, String>,
    input_rate: f64,
    output_rate: f64,
) {
    let (Some(app), Some(audit)) = (app, audit) else {
        return;
    };
    let (status, error_code, input_tokens, output_tokens, output_digest) = match result {
        Ok(response) => {
            let content = response
                .choices
                .first()
                .map(|choice| choice.message.content.as_str())
                .unwrap_or_default();
            (
                "success",
                None,
                response
                    .usage
                    .as_ref()
                    .map_or(0, |usage| usage.prompt_tokens),
                response
                    .usage
                    .as_ref()
                    .map_or(0, |usage| usage.completion_tokens),
                crate::content_digest::digest(app, &[content.as_bytes()]).ok(),
            )
        }
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "AI_PROVIDER_CALL_FAILED",
            )),
            0,
            0,
            None,
        ),
    };
    let estimated_cost =
        (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0;
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "ai".to_owned(),
            actor_id: "supervisor".to_owned(),
            category: "ai".to_owned(),
            action: "supervisor_provider_call_completed".to_owned(),
            scope_type: super::audit_scope_type(&context.board_id).to_owned(),
            scope_id: context.board_id.clone(),
            session_id: Some(audit.session_id.clone()),
            session_kind: Some("ai_provider".to_owned()),
            session_title: Some("Supervisor AI 会话".to_owned()),
            end_session: true,
            correlation_id: Some(context.task_id.clone()),
            causation_id: Some(audit.intent_id.clone()),
            result: status.to_owned(),
            error_code,
            summary: "Supervisor AI Provider 会话结束".to_owned(),
            metadata: json!({"runId": run_id, "roundIndex": round_index, "providerId": provider_id, "callType": "supervisor_decision", "latencyMs": audit.started.elapsed().as_millis(), "usageAvailable": result.as_ref().is_ok_and(|response| response.usage.is_some()), "inputTokens": input_tokens, "outputTokens": output_tokens, "estimatedCost": estimated_cost, "inputDigest": audit.input_digest, "outputDigest": output_digest, "digestAlgorithm": "hmac-sha256-v1"}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let sensitive = [
                        "api_key",
                        "apikey",
                        "password",
                        "authorization",
                        "private_key",
                        "secret",
                    ]
                    .iter()
                    .any(|marker| normalized.contains(marker))
                        || matches!(
                            normalized.as_str(),
                            "token"
                                | "access_token"
                                | "refresh_token"
                                | "bearer_token"
                                | "id_token"
                        );
                    (
                        key.clone(),
                        if sensitive {
                            Value::String("[REDACTED]".to_owned())
                        } else {
                            redact_value(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        Value::String(value) => Value::String(redact_text(value)),
        other => other.clone(),
    }
}

fn context_json(context: &FailureContext, allowed_actions: &[String], remaining: Value) -> Value {
    redact_value(&json!({
        "task_id": context.task_id,
        "board_id": context.board_id,
        "task_kind": context.task_kind,
        "step_name": context.step_name,
        "failure_type": context.failure_type,
        "error_code": context.error_code,
        "key_logs": context.key_logs.iter().take(80).collect::<Vec<_>>(),
        "device_state": context.device_state,
        "firmware_hash": context.firmware_hash,
        "allowed_actions": allowed_actions,
        "remaining_budget": remaining,
        "decision_schema": {
            "decision": "allowed action name",
            "reason": "inference based on cited evidence",
            "confidence": "number 0..1",
            "evidence": ["exact fact from context"],
            "parameters": {},
            "expected_result": "deterministic observable result",
            "need_human": false,
            "human_instruction": null
        }
    }))
}

fn parse_json_content(content: &str) -> Result<SupervisorDecision, String> {
    let trimmed = content.trim();
    let json_text = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    serde_json::from_str(json_text).map_err(|error| format!("SUPERVISOR_DECISION_JSON:{error}"))
}

fn fingerprint(context: &FailureContext) -> String {
    let mut digest = Sha256::new();
    digest.update(context.board_id.as_bytes());
    digest.update([0]);
    digest.update(context.step_name.as_bytes());
    digest.update([0]);
    digest.update(context.failure_type.as_bytes());
    digest.update([0]);
    if let Some(code) = &context.error_code {
        digest.update(code.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn run_id() -> String {
    format!(
        "supervisor-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    )
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub async fn analyze(
    app: Option<&tauri::AppHandle>,
    state: &Arc<SupervisorState>,
    context: FailureContext,
) -> Result<SupervisorAnalysisResult, String> {
    if context.task_id.trim().is_empty()
        || context.board_id.trim().is_empty()
        || context.policy_id <= 0
    {
        return Err("SUPERVISOR_CONTEXT_SCOPE_REQUIRED".to_owned());
    }
    let current_run_id = context.run_id.clone().unwrap_or_else(run_id);
    let (policy, round_index, redacted, fingerprint) = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let policy = storage::policy(&db, context.policy_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_POLICY_NOT_FOUND".to_owned())?;
        if !policy.enabled {
            return Err("SUPERVISOR_POLICY_DISABLED".to_owned());
        }
        storage::create_or_get_run(
            &db,
            &current_run_id,
            &context.task_id,
            &context.board_id,
            context.policy_id,
        )
        .map_err(|error| error.to_string())?;
        let scope = storage::run_scope(&db, &current_run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
        if scope
            != (
                context.task_id.clone(),
                context.board_id.clone(),
                context.policy_id,
            )
        {
            return Err("SUPERVISOR_RUN_SCOPE_MISMATCH".to_owned());
        }
        let budget = storage::run_budget(&db, &current_run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
        if !matches!(budget.status.as_str(), "running" | "waiting_approval") {
            return Err(format!("SUPERVISOR_RUN_NOT_ACTIVE:{}", budget.status));
        }
        if now().saturating_sub(budget.started_at) >= i64::from(policy.max_duration_seconds) {
            storage::create_human_request(
                &db,
                &current_run_id,
                "监督时长预算已用尽",
                "请检查现有证据后选择继续、手动接管或终止任务。",
                &context.key_logs,
            )
            .map_err(|error| error.to_string())?;
            return Err("SUPERVISOR_DURATION_BUDGET_EXHAUSTED".to_owned());
        }
        let round_index =
            storage::round_count(&db, &current_run_id).map_err(|error| error.to_string())? + 1;
        if round_index > policy.max_attempts {
            storage::create_human_request(
                &db,
                &current_run_id,
                "自主尝试次数已用尽",
                "请检查监督时间线后选择继续、手动接管或终止任务。",
                &context.key_logs,
            )
            .map_err(|error| error.to_string())?;
            return Err("SUPERVISOR_ATTEMPT_BUDGET_EXHAUSTED".to_owned());
        }
        let remaining = json!({
            "attempts": policy.max_attempts - round_index,
            "seconds": i64::from(policy.max_duration_seconds) - now().saturating_sub(budget.started_at),
            "power_cycles": policy.max_power_cycles.saturating_sub(budget.power_cycle_count),
            "flash_retries": policy.max_flash_retries.saturating_sub(budget.flash_retry_count),
            "cost": policy.cost_limit.map(|limit| (limit - budget.estimated_cost).max(0.0)),
            "tokens_used": budget.total_input_tokens + budget.total_output_tokens
        });
        let allowed = policy
            .allowed_actions
            .iter()
            .chain(policy.approval_actions.iter())
            .cloned()
            .collect::<Vec<_>>();
        let redacted = context_json(&context, &allowed, remaining);
        let fingerprint = fingerprint(&context);
        (policy, round_index, redacted, fingerprint)
    };

    let (provider, api_key) = if let Some(provider_id) = policy.provider_id {
        let provider = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::provider(&db, provider_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "AI_PROVIDER_NOT_FOUND".to_owned())?
        };
        if !provider.enabled {
            return Err("AI_PROVIDER_DISABLED".to_owned());
        }
        let api_key = super::credentials::resolve(provider_id)?;
        (provider, api_key)
    } else {
        resolve_default_provider(state)?
    };
    let provider_id = provider
        .id
        .ok_or_else(|| "AI_PROVIDER_ID_INVALID".to_owned())?;
    let effective_cost_limit = match (policy.cost_limit, provider.task_cost_limit) {
        (Some(policy_limit), Some(provider_limit)) => Some(policy_limit.min(provider_limit)),
        (Some(limit), None) | (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    if effective_cost_limit.is_some()
        && provider.input_cost_per_million.is_none()
        && provider.output_cost_per_million.is_none()
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::create_human_request(
            &db,
            &current_run_id,
            "费用预算无法估算",
            "请在 AI 服务连接中填写每百万输入/输出 Token 费率，或取消费用上限。",
            &context.key_logs,
        )
        .map_err(|error| error.to_string())?;
        return Err("AI_PROVIDER_TOKEN_PRICE_REQUIRED_FOR_COST_LIMIT".to_owned());
    }
    if let Some(limit) = effective_cost_limit {
        let spent = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::run_budget(&db, &current_run_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?
                .estimated_cost
        };
        if spent >= limit {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::create_human_request(
                &db,
                &current_run_id,
                "AI 调用费用预算已用尽",
                "请调整连接或策略费用上限，或转为手动处理。",
                &context.key_logs,
            )
            .map_err(|error| error.to_string())?;
            return Err("SUPERVISOR_COST_BUDGET_EXHAUSTED".to_owned());
        }
    }
    let request_max_tokens = if let Some(max_tokens) = provider.max_tokens {
        let used = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            let budget = storage::run_budget(&db, &current_run_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
            budget.total_input_tokens + budget.total_output_tokens
        };
        if used >= u64::from(max_tokens) {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::create_human_request(
                &db,
                &current_run_id,
                "Token 预算已用尽",
                "请调整连接的 Token 上限或转为手动处理。",
                &context.key_logs,
            )
            .map_err(|error| error.to_string())?;
            return Err("SUPERVISOR_TOKEN_BUDGET_EXHAUSTED".to_owned());
        }
        Some(
            u64::from(max_tokens)
                .saturating_sub(used)
                .min(u64::from(u32::MAX)) as u32,
        )
    } else {
        None
    };
    let request = CompletionRequest {
        model: provider.model.clone(),
        messages: vec![
            CompletionMessage {
                role: "system",
                content: SYSTEM_PROMPT.to_owned(),
            },
            CompletionMessage {
                role: "user",
                content: redacted.to_string(),
            },
        ],
        max_tokens: request_max_tokens,
        stream: false,
    };
    let provider_audit = begin_provider_call_audit(
        app,
        &context,
        &current_run_id,
        round_index,
        provider_id,
        &redacted.to_string(),
    )?;
    let raw_response = state
        .http
        .post(endpoint(&provider.base_url, "chat/completions")?)
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_secs(provider.timeout_seconds))
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "AI_PROVIDER_TIMEOUT".to_owned()
            } else {
                "AI_PROVIDER_NETWORK_ERROR".to_owned()
            }
        });
    let response_result = match raw_response {
        Ok(response) if response.status().is_success() => response
            .json::<CompletionResponse>()
            .await
            .map_err(|_| "AI_PROVIDER_RESPONSE_INVALID".to_owned()),
        Ok(response) => Err(safe_http_message(response.status())),
        Err(error) => Err(error),
    };
    finish_provider_call_audit(
        app,
        &context,
        &current_run_id,
        round_index,
        provider_id,
        provider_audit.as_ref(),
        &response_result,
        provider.input_cost_per_million.unwrap_or(0.0),
        provider.output_cost_per_million.unwrap_or(0.0),
    );
    let response = response_result?;
    let usage = response.usage.as_ref().cloned().unwrap_or_default();
    let content = response
        .choices
        .first()
        .map(|choice| choice.message.content.as_str())
        .ok_or_else(|| "AI_PROVIDER_DECISION_MISSING".to_owned())?;
    let decision = parse_json_content(content)?;
    let estimated_cost = (usage.prompt_tokens as f64
        * provider.input_cost_per_million.unwrap_or(0.0)
        + usage.completion_tokens as f64 * provider.output_cost_per_million.unwrap_or(0.0))
        / 1_000_000.0;
    let combined_actions = policy
        .allowed_actions
        .iter()
        .chain(policy.approval_actions.iter())
        .cloned()
        .collect::<Vec<_>>();
    validate_decision(&decision, &combined_actions)?;

    let repeated_before = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::repeated_decision_count(&db, &current_run_id, &fingerprint, &decision.decision)
            .map_err(|error| error.to_string())?
    };
    let action_budget_exhausted = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let budget = storage::run_budget(&db, &current_run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
        (decision.decision == "power_cycle" && budget.power_cycle_count >= policy.max_power_cycles)
            || (decision.decision == "retry_flash"
                && budget.flash_retry_count >= policy.max_flash_retries)
    };
    let force_approval = matches!(decision.decision.as_str(), "retry_flash" | "retry_step");
    let approval_required = force_approval || policy.approval_actions.contains(&decision.decision);
    let policy_result = if action_budget_exhausted {
        "rejected_budget"
    } else if repeated_before >= policy.max_same_error {
        "rejected_no_progress"
    } else if decision.need_human || decision.decision == "request_human" {
        "human_required"
    } else if decision.decision == "finish_success" {
        "verification_required"
    } else if decision.decision == "finish_failed" {
        "final_failed"
    } else if approval_required {
        "approval_required"
    } else {
        "allowed"
    };
    let execution_allowed = policy_result == "allowed";
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::insert_round(
            &db,
            RoundRecord {
                run_id: &current_run_id,
                round_index,
                fingerprint: &fingerprint,
                context: &redacted,
                decision: &decision,
                schema_result: "valid",
                policy_result,
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                estimated_cost,
            },
        )
        .map_err(|error| error.to_string())?;
        storage::insert_audit(
            &db,
            storage::AuditInsert {
                run_id: &current_run_id,
                board_id: &context.board_id,
                provider_id,
                result: policy_result,
                context: &redacted,
                decision: &decision,
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                estimated_cost,
            },
        )
        .map_err(|error| error.to_string())?;
        if matches!(
            policy_result,
            "human_required" | "rejected_no_progress" | "rejected_budget"
        ) {
            storage::create_human_request(
                &db,
                &current_run_id,
                &decision.reason,
                decision.human_instruction.as_deref().unwrap_or(
                    "请检查板卡连接、供电、启动模式和当前日志，然后选择继续或转为手动。",
                ),
                &decision.evidence,
            )
            .map_err(|error| error.to_string())?;
        }
        if policy_result == "final_failed" {
            storage::set_run_status(&db, &current_run_id, "failed", &decision.reason)
                .map_err(|error| error.to_string())?;
        }
    }
    let result = SupervisorAnalysisResult {
        run_id: current_run_id,
        round_index,
        decision,
        policy_result: policy_result.to_owned(),
        execution_allowed,
        approval_required,
        redacted_context: redacted,
    };
    if let Some(app) = app {
        super::audit_policy_decision(app, &context, &result);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an explicitly configured live supervisor database and keyring"]
    async fn live_provider_supervisor_round() {
        let database = std::env::var("LYNX_LIVE_SUPERVISOR_DB")
            .expect("LYNX_LIVE_SUPERVISOR_DB must point to the live test database");
        let policy_id = std::env::var("LYNX_LIVE_SUPERVISOR_POLICY_ID")
            .expect("LYNX_LIVE_SUPERVISOR_POLICY_ID is required")
            .parse::<i64>()
            .expect("policy id must be an integer");
        let expected_action =
            std::env::var("LYNX_LIVE_EXPECTED_ACTION").unwrap_or_else(|_| "rescan_usb".to_owned());
        let expected_policy_result = std::env::var("LYNX_LIVE_EXPECTED_POLICY_RESULT")
            .unwrap_or_else(|_| "allowed".to_owned());
        let state = Arc::new(
            SupervisorState::open(std::path::Path::new(&database))
                .expect("live supervisor database must open"),
        );
        let result = analyze(
            None,
            &state,
            FailureContext {
                run_id: None,
                task_id: format!("live-test-{}", now()),
                board_id: "linux:4".to_owned(),
                policy_id,
                task_kind: "flash".to_owned(),
                step_name: "device_scan".to_owned(),
                failure_type: "DEVICE_NOT_FOUND".to_owned(),
                error_code: Some("OPENIX_DEVICE_NOT_FOUND".to_owned()),
                key_logs: vec![
                    "openixcli could not find the selected USB device".to_owned(),
                    "api_key=sk-live-redaction-123456789".to_owned(),
                ],
                device_state: json!({"binding":"3:2", "last_scan":"not_found"}),
                firmware_hash: Some("live-test-no-flash".to_owned()),
            },
        )
        .await
        .expect("live provider should return a valid supervisor decision");
        assert_eq!(result.decision.decision, expected_action);
        assert_eq!(result.policy_result, expected_policy_result);
        assert_eq!(result.execution_allowed, result.policy_result == "allowed");
        assert!(!result
            .redacted_context
            .to_string()
            .contains("sk-live-redaction"));
        eprintln!(
            "live supervisor run={} round={} decision={}",
            result.run_id, result.round_index, result.decision.decision
        );
    }

    #[tokio::test]
    #[ignore = "requires a live provider policy whose max_attempts is one"]
    async fn live_attempt_budget_creates_human_request() {
        let database = std::env::var("LYNX_LIVE_SUPERVISOR_DB")
            .expect("LYNX_LIVE_SUPERVISOR_DB must point to the live test database");
        let policy_id = std::env::var("LYNX_LIVE_SUPERVISOR_POLICY_ID")
            .expect("LYNX_LIVE_SUPERVISOR_POLICY_ID is required")
            .parse::<i64>()
            .expect("policy id must be an integer");
        let expected_error = std::env::var("LYNX_LIVE_EXPECTED_BUDGET_ERROR")
            .unwrap_or_else(|_| "SUPERVISOR_ATTEMPT_BUDGET_EXHAUSTED".to_owned());
        let state = Arc::new(
            SupervisorState::open(std::path::Path::new(&database))
                .expect("live supervisor database must open"),
        );
        let mut context = FailureContext {
            run_id: None,
            task_id: format!("live-budget-test-{}", now()),
            board_id: "linux:4".to_owned(),
            policy_id,
            task_kind: "flash".to_owned(),
            step_name: "device_scan".to_owned(),
            failure_type: "DEVICE_NOT_FOUND".to_owned(),
            error_code: Some("OPENIX_DEVICE_NOT_FOUND".to_owned()),
            key_logs: vec!["selected T153 USB binding was not found".to_owned()],
            device_state: json!({"binding":"3:2", "last_scan":"not_found"}),
            firmware_hash: Some("live-test-no-flash".to_owned()),
        };
        let first = analyze(None, &state, context.clone())
            .await
            .expect("first attempt should call the live provider");
        context.run_id = Some(first.run_id.clone());
        let second = analyze(None, &state, context)
            .await
            .expect_err("second attempt must stop before another provider call");
        assert_eq!(second, expected_error);
        let db = state.db.lock().unwrap();
        let requests = storage::list_human_requests(&db, true).unwrap();
        assert!(requests
            .iter()
            .any(|request| request.run_id == first.run_id));
    }

    #[test]
    fn redaction_removes_nested_secrets() {
        let value = json!({"apiKey":"secret", "nested":{"authorization":"Bearer x"}, "safe":"Authorization: Bearer sk-1234567890", "tokens_used":42});
        let redacted = redact_value(&value);
        assert_eq!(redacted["apiKey"], "[REDACTED]");
        assert_eq!(redacted["nested"]["authorization"], "[REDACTED]");
        assert_eq!(redacted["safe"], "Authorization: Bearer [REDACTED]");
        assert_eq!(redacted["tokens_used"], 42);
    }

    #[test]
    fn fenced_json_decision_is_accepted() {
        let decision = parse_json_content("```json\n{\"decision\":\"collect_logs\",\"reason\":\"need facts\",\"confidence\":0.5,\"evidence\":[\"timeout\"],\"parameters\":{},\"expected_result\":\"logs\",\"need_human\":false,\"human_instruction\":null}\n```").unwrap();
        assert_eq!(decision.decision, "collect_logs");
    }
}

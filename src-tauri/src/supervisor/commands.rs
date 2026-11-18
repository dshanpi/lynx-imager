use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tauri::{AppHandle, State};
use url::Url;

use super::credentials;
use super::engine;
use super::executor::{self, ActionExecutionResult};
use super::scheduler;
use super::storage;
use super::types::{
    AiProviderConfig, AutomaticTask, FailureContext, HumanInterventionRequest, ProviderTestResult,
    SaveAiProviderRequest, SupervisorAnalysisResult, SupervisorAuditRecord, SupervisorOverview,
    SupervisorPolicy, SupervisorRun, SUPERVISOR_ACTIONS,
};
use super::SupervisorState;

const PROVIDER_AUDIT_FIELDS: &[&str] = &[
    "name",
    "preset",
    "baseUrl",
    "model",
    "timeoutSeconds",
    "maxTokens",
    "taskCostLimit",
    "inputCostPerMillion",
    "outputCostPerMillion",
    "enabled",
    "isDefault",
    "credential",
];

fn provider_audit_value(provider: &AiProviderConfig) -> Value {
    json!({
        "name": provider.name, "preset": provider.preset, "baseUrl": provider.base_url,
        "model": provider.model, "timeoutSeconds": provider.timeout_seconds,
        "maxTokens": provider.max_tokens, "taskCostLimit": provider.task_cost_limit,
        "inputCostPerMillion": provider.input_cost_per_million,
        "outputCostPerMillion": provider.output_cost_per_million, "enabled": provider.enabled,
        "isDefault": provider.is_default, "credential": provider.has_api_key,
    })
}

fn provider_request_audit_value(request: &SaveAiProviderRequest) -> Value {
    json!({
        "name": request.name, "preset": request.preset, "baseUrl": request.base_url,
        "model": request.model, "timeoutSeconds": request.timeout_seconds,
        "maxTokens": request.max_tokens, "taskCostLimit": request.task_cost_limit,
        "inputCostPerMillion": request.input_cost_per_million,
        "outputCostPerMillion": request.output_cost_per_million, "enabled": request.enabled,
        "isDefault": request.is_default, "credential": request.api_key.as_ref().is_some_and(|value| !value.trim().is_empty()),
    })
}

fn validate_provider(request: &SaveAiProviderRequest) -> Result<(), String> {
    if request.name.trim().is_empty() || request.model.trim().is_empty() {
        return Err("AI_PROVIDER_NAME_AND_MODEL_REQUIRED".to_owned());
    }
    if !(5..=600).contains(&request.timeout_seconds) {
        return Err("AI_PROVIDER_TIMEOUT_OUT_OF_RANGE".to_owned());
    }
    if request
        .max_tokens
        .is_some_and(|tokens| tokens == 0 || tokens > 1_000_000)
    {
        return Err("AI_PROVIDER_MAX_TOKENS_OUT_OF_RANGE".to_owned());
    }
    if request
        .task_cost_limit
        .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
    {
        return Err("AI_PROVIDER_COST_LIMIT_INVALID".to_owned());
    }
    for rate in [
        request.input_cost_per_million,
        request.output_cost_per_million,
    ] {
        if rate.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err("AI_PROVIDER_TOKEN_PRICE_INVALID".to_owned());
        }
    }
    if request.task_cost_limit.is_some()
        && request.input_cost_per_million.is_none()
        && request.output_cost_per_million.is_none()
    {
        return Err("AI_PROVIDER_TOKEN_PRICE_REQUIRED_FOR_COST_LIMIT".to_owned());
    }
    validate_base_url(&request.base_url)?;
    Ok(())
}

fn validate_base_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "AI_PROVIDER_URL_INVALID".to_owned())?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("AI_PROVIDER_URL_MUST_NOT_CONTAIN_CREDENTIALS_QUERY_OR_FRAGMENT".to_owned());
    }
    let host = url.host_str().unwrap_or_default();
    let private_http = host == "localhost"
        || host.parse::<IpAddr>().is_ok_and(|address| match address {
            IpAddr::V4(address) => address.is_loopback() || address.is_private(),
            IpAddr::V6(address) => address.is_loopback() || address.is_unique_local(),
        });
    if url.scheme() != "https" && !(url.scheme() == "http" && private_http) {
        return Err("AI_PROVIDER_HTTPS_REQUIRED_FOR_PUBLIC_HOST".to_owned());
    }
    Ok(url)
}

pub(crate) fn endpoint(base_url: &str, suffix: &str) -> Result<String, String> {
    let mut value = validate_base_url(base_url)?.to_string();
    while value.ends_with('/') {
        value.pop();
    }
    Ok(format!("{value}/{suffix}"))
}

pub(crate) fn safe_http_message(status: reqwest::StatusCode) -> String {
    let generic = match status.as_u16() {
        401 | 403 => "认证失败，请检查 API Key",
        429 => "请求频率或额度受限",
        500..=599 => "服务端暂时不可用",
        _ => "兼容接口返回错误",
    };
    // A provider error body is untrusted and may echo request headers or
    // gateway diagnostics. Never surface it into UI, logs or audit records.
    format!("HTTP {status}: {generic}")
}

#[tauri::command]
pub fn supervisor_list_providers(
    state: State<'_, Arc<SupervisorState>>,
) -> Result<Vec<AiProviderConfig>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_providers(&db).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn supervisor_save_provider(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    request: SaveAiProviderRequest,
) -> Result<AiProviderConfig, String> {
    validate_provider(&request)?;
    let secret = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let old = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        request
            .id
            .map(|id| storage::provider(&db, id))
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten()
    };
    let old_value = old.as_ref().map(provider_audit_value);
    let new_value = provider_request_audit_value(&request);
    let fields = crate::config_audit::changed_fields(
        old_value.as_ref(),
        Some(&new_value),
        PROVIDER_AUDIT_FIELDS,
    );
    let operation = if request.id.is_some() {
        "update"
    } else {
        "create"
    };
    let scope_id = request.id.map_or_else(
        || format!("ai-provider-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("ai-provider:{id}"),
    );
    let audit = crate::config_audit::begin(
        &app,
        "ai_provider",
        scope_id,
        "ai_provider",
        operation,
        fields,
        json!({"preset": request.preset, "enabled": request.enabled, "default": request.is_default, "credentialChanged": secret.is_some()}),
    )?;
    let previous_secret = request.id.and_then(|id| credentials::resolve(id).ok());
    let result = (|| {
        let mut db = state.db.lock().map_err(|error| error.to_string())?;
        let transaction = db.transaction().map_err(|error| error.to_string())?;
        let id =
            storage::save_provider(&transaction, &request).map_err(|error| error.to_string())?;
        if let Some(api_key) = secret {
            credentials::save(id, api_key)?;
        } else if request.id.is_none() {
            return Err("AI_CREDENTIAL_REQUIRED_FOR_NEW_PROVIDER".to_owned());
        }
        if let Err(error) = transaction.commit() {
            let compensated = match previous_secret.as_deref() {
                Some(value) => credentials::save(id, value),
                None => credentials::delete(id),
            };
            return if compensated.is_ok() {
                Err(error.to_string())
            } else {
                Err("CONFIG_COMPENSATION_FAILED".to_owned())
            };
        }
        storage::provider(&db, id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "AI_PROVIDER_NOT_FOUND_AFTER_SAVE".to_owned())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub fn supervisor_delete_provider(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    provider_id: i64,
) -> Result<(), String> {
    if provider_id <= 0 {
        return Err("AI_PROVIDER_ID_INVALID".to_owned());
    }
    let audit = crate::config_audit::begin(
        &app,
        "ai_provider",
        format!("ai-provider:{provider_id}"),
        "ai_provider",
        "delete",
        vec![],
        json!({}),
    )?;
    let previous_secret = credentials::resolve(provider_id).ok();
    let result = (|| {
        credentials::delete(provider_id)?;
        let db = state.db.lock().map_err(|error| error.to_string())?;
        if let Err(error) = storage::delete_provider(&db, provider_id) {
            return match previous_secret.as_deref() {
                Some(value) if credentials::save(provider_id, value).is_err() => {
                    Err("CONFIG_COMPENSATION_FAILED".to_owned())
                }
                _ => Err(error.to_string()),
            };
        }
        Ok(())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub async fn supervisor_test_provider(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    provider_id: i64,
) -> Result<ProviderTestResult, String> {
    if provider_id <= 0 {
        return Err("AI_PROVIDER_ID_INVALID".to_owned());
    }
    let session_id = format!("provider-test-{}-{:016x}", now_ms(), rand::random::<u64>());
    let outcome = crate::audit::publish_required(
        &app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "connection".to_owned(),
            action: "ai_provider_test_intended".to_owned(),
            scope_type: "ai_provider".to_owned(),
            scope_id: format!("ai-provider:{provider_id}"),
            session_id: Some(session_id.clone()),
            session_kind: Some("connection_test".to_owned()),
            session_title: Some("AI 服务连接测试".to_owned()),
            end_session: false,
            correlation_id: Some(session_id.clone()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备测试 AI 服务连接".to_owned(),
            metadata: json!({"providerId": provider_id, "testType": "provider"}),
        },
    )?;
    let crate::audit::AuditWriteOutcome::Recorded { event_id } = outcome else {
        return Err("AUDIT_REQUIRED_DEGRADED".to_owned());
    };
    let result = test_provider_core(state.inner().as_ref(), provider_id, &session_id).await;
    let (status, code, latency) = match &result {
        Ok(value) => (
            if value.ok { "success" } else { "failed" },
            if value.ok {
                None
            } else {
                Some(format!(
                    "AI_PROVIDER_TEST_{}",
                    value.status.to_ascii_uppercase()
                ))
            },
            value.latency_ms,
        ),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "AI_PROVIDER_TEST_FAILED",
            )),
            0,
        ),
    };
    let _ = crate::audit::publish(
        &app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "connection".to_owned(),
            action: "ai_provider_test_completed".to_owned(),
            scope_type: "ai_provider".to_owned(),
            scope_id: format!("ai-provider:{provider_id}"),
            session_id: Some(session_id.clone()),
            session_kind: Some("connection_test".to_owned()),
            session_title: Some("AI 服务连接测试".to_owned()),
            end_session: true,
            correlation_id: Some(session_id),
            causation_id: Some(event_id),
            result: status.to_owned(),
            error_code: code,
            summary: "AI 服务连接测试结束".to_owned(),
            metadata: json!({"providerId": provider_id, "testType": "provider", "latencyMs": latency}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
    result
}

async fn test_provider_core(
    state: &SupervisorState,
    provider_id: i64,
    audit_session_id: &str,
) -> Result<ProviderTestResult, String> {
    let provider = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::provider(&db, provider_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "AI_PROVIDER_NOT_FOUND".to_owned())?
    };
    let api_key = credentials::resolve(provider_id)?;
    let url = endpoint(&provider.base_url, "models")?;
    let started = Instant::now();
    let models_response = state
        .http
        .get(&url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(provider.timeout_seconds))
        .send()
        .await;
    let models_supported = match models_response {
        Ok(response) if response.status().is_success() => {
            if response.json::<Value>().await.is_err() {
                let result = ProviderTestResult {
                    ok: false,
                    status: "invalid_response".to_owned(),
                    message: "模型列表接口成功但返回内容不是 JSON".to_owned(),
                    latency_ms: started.elapsed().as_millis(),
                    models_endpoint: url,
                    audit_session_id: audit_session_id.to_owned(),
                };
                let db = state.db.lock().map_err(|error| error.to_string())?;
                storage::update_test_result(&db, provider_id, &result.status, &result.message)
                    .map_err(|error| error.to_string())?;
                return Ok(result);
            }
            true
        }
        Ok(response) if matches!(response.status().as_u16(), 404 | 405) => false,
        Ok(response) => {
            let status = response.status();
            let category = if matches!(status.as_u16(), 401 | 403) {
                "authentication_failed"
            } else if status.as_u16() == 429 {
                "rate_limited"
            } else {
                "http_error"
            };
            let message = safe_http_message(status);
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::update_test_result(&db, provider_id, category, &message)
                .map_err(|error| error.to_string())?;
            return Ok(ProviderTestResult {
                ok: false,
                status: category.to_owned(),
                message,
                latency_ms: started.elapsed().as_millis(),
                models_endpoint: url,
                audit_session_id: audit_session_id.to_owned(),
            });
        }
        Err(error) if error.is_timeout() => {
            return provider_test_failure(
                state,
                provider_id,
                started,
                url,
                "timeout",
                "连接测试超时",
                audit_session_id,
            );
        }
        Err(_) => {
            return provider_test_failure(
                state,
                provider_id,
                started,
                url,
                "network_error",
                "无法连接兼容接口",
                audit_session_id,
            );
        }
    };

    // A successful /models response proves that the key and endpoint work, but
    // not that the configured model accepts chat completions. Always perform a
    // minimal completion so aliases and custom gateways are tested end to end.
    let chat_url = endpoint(&provider.base_url, "chat/completions")?;
    let response = state
        .http
        .post(&chat_url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(provider.timeout_seconds))
        .json(&serde_json::json!({
            "model": provider.model,
            "messages": [{"role": "user", "content": "Reply OK"}],
            "max_tokens": 1,
            "stream": false
        }))
        .send()
        .await;
    let (status, message, ok) = match response {
        Ok(response) if response.status().is_success() => {
            if response.json::<Value>().await.is_ok() {
                let suffix = if models_supported {
                    ""
                } else {
                    "（服务未提供模型列表）"
                };
                (
                    "available",
                    format!("连接成功，配置模型可调用{suffix}"),
                    true,
                )
            } else {
                (
                    "invalid_response",
                    "模型调用成功但返回内容不是 JSON".to_owned(),
                    false,
                )
            }
        }
        Ok(response) => {
            let status = response.status();
            let category = if matches!(status.as_u16(), 401 | 403) {
                "authentication_failed"
            } else if status.as_u16() == 429 {
                "rate_limited"
            } else {
                "model_call_failed"
            };
            (category, safe_http_message(status), false)
        }
        Err(error) if error.is_timeout() => ("timeout", "模型调用测试超时".to_owned(), false),
        Err(_) => ("network_error", "无法调用配置模型".to_owned(), false),
    };
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::update_test_result(&db, provider_id, status, &message)
            .map_err(|error| error.to_string())?;
    }
    Ok(ProviderTestResult {
        ok,
        status: status.to_owned(),
        message,
        latency_ms: started.elapsed().as_millis(),
        models_endpoint: url,
        audit_session_id: audit_session_id.to_owned(),
    })
}

fn provider_test_failure(
    state: &SupervisorState,
    provider_id: i64,
    started: Instant,
    models_endpoint: String,
    status: &str,
    message: &str,
    audit_session_id: &str,
) -> Result<ProviderTestResult, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::update_test_result(&db, provider_id, status, message)
        .map_err(|error| error.to_string())?;
    Ok(ProviderTestResult {
        ok: false,
        status: status.to_owned(),
        message: message.to_owned(),
        latency_ms: started.elapsed().as_millis(),
        models_endpoint,
        audit_session_id: audit_session_id.to_owned(),
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[tauri::command]
pub fn supervisor_list_policies(
    state: State<'_, Arc<SupervisorState>>,
) -> Result<Vec<SupervisorPolicy>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_policies(&db).map_err(|error| error.to_string())
}

fn validate_policy(policy: &SupervisorPolicy) -> Result<(), String> {
    if policy.name.trim().is_empty() {
        return Err("SUPERVISOR_POLICY_NAME_REQUIRED".to_owned());
    }
    if policy.max_attempts == 0
        || policy.max_attempts > 50
        || policy.max_duration_seconds < 30
        || policy.max_duration_seconds > 86_400
        || policy.max_same_error == 0
        || policy.max_power_cycles > 20
        || policy.max_flash_retries > 20
        || policy.wait_device_seconds == 0
        || policy.wait_device_seconds > 3_600
    {
        return Err("SUPERVISOR_POLICY_BUDGET_INVALID".to_owned());
    }
    if policy
        .cost_limit
        .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
    {
        return Err("SUPERVISOR_POLICY_COST_LIMIT_INVALID".to_owned());
    }
    for action in policy
        .allowed_actions
        .iter()
        .chain(policy.approval_actions.iter())
    {
        if !SUPERVISOR_ACTIONS.contains(&action.as_str()) {
            return Err(format!("SUPERVISOR_UNKNOWN_ACTION:{action}"));
        }
    }
    if policy
        .approval_actions
        .iter()
        .any(|action| policy.allowed_actions.contains(action))
    {
        return Err("SUPERVISOR_ACTION_PERMISSION_OVERLAP".to_owned());
    }
    Ok(())
}

fn conflicting_policy_board(
    policy: &SupervisorPolicy,
    existing: &[SupervisorPolicy],
) -> Option<String> {
    if !policy.enabled {
        return None;
    }
    existing
        .iter()
        .filter(|item| item.enabled && item.id != policy.id)
        .flat_map(|item| item.board_scope.iter())
        .find(|board| policy.board_scope.contains(board))
        .cloned()
}

#[tauri::command]
pub fn supervisor_save_policy(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
    policy: SupervisorPolicy,
) -> Result<SupervisorPolicy, String> {
    validate_policy(&policy)?;
    let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    storage::sync_board_identities(&mut db, &workbench_db).map_err(|error| error.to_string())?;
    drop(workbench_db);
    let existing = storage::list_policies(&db).map_err(|error| error.to_string())?;
    if let Some(board) = conflicting_policy_board(&policy, &existing) {
        return Err(format!("SUPERVISOR_POLICY_BOARD_SCOPE_CONFLICT:{board}"));
    }
    let old = policy
        .id
        .and_then(|id| existing.iter().find(|item| item.id == Some(id)))
        .and_then(|value| serde_json::to_value(value).ok());
    let new = serde_json::to_value(&policy).map_err(|_| "SUPERVISOR_POLICY_ENCODE_FAILED")?;
    let fields = crate::config_audit::changed_fields(
        old.as_ref(),
        Some(&new),
        &[
            "name",
            "enabled",
            "providerId",
            "maxAttempts",
            "maxDurationSeconds",
            "maxSameError",
            "maxPowerCycles",
            "maxFlashRetries",
            "waitDeviceSeconds",
            "costLimit",
            "allowedActions",
            "approvalActions",
            "boardScope",
            "successSummary",
            "notificationMode",
        ],
    );
    let operation = if policy.id.is_some() {
        "update"
    } else {
        "create"
    };
    let scope_id = policy.id.map_or_else(
        || format!("policy-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("policy:{id}"),
    );
    drop(db);
    let audit = crate::config_audit::begin(
        &app,
        "supervisor_policy",
        scope_id,
        "supervisor_policy",
        operation,
        fields,
        json!({"enabled": policy.enabled, "boardScopeCount": policy.board_scope.len(), "allowedActionCount": policy.allowed_actions.len(), "approvalActionCount": policy.approval_actions.len(), "notificationMode": policy.notification_mode}),
    )?;
    let result = (|| {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let existing = storage::list_policies(&db).map_err(|error| error.to_string())?;
        if let Some(board) = conflicting_policy_board(&policy, &existing) {
            return Err(format!("SUPERVISOR_POLICY_BOARD_SCOPE_CONFLICT:{board}"));
        }
        let id = storage::save_policy(&db, &policy).map_err(|error| error.to_string())?;
        storage::list_policies(&db)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|item| item.id == Some(id))
            .ok_or_else(|| "SUPERVISOR_POLICY_NOT_FOUND_AFTER_SAVE".to_owned())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub fn supervisor_delete_policy(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    policy_id: i64,
) -> Result<(), String> {
    if policy_id <= 0 {
        return Err("SUPERVISOR_POLICY_ID_INVALID".to_owned());
    }
    let audit = crate::config_audit::begin(
        &app,
        "supervisor_policy",
        format!("policy:{policy_id}"),
        "supervisor_policy",
        "delete",
        vec![],
        json!({}),
    )?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let result = storage::delete_policy(&db, policy_id).map_err(|error| error.to_string());
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub fn supervisor_overview(
    state: State<'_, Arc<SupervisorState>>,
) -> Result<SupervisorOverview, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::overview(&db).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn supervisor_list_audit(
    state: State<'_, Arc<SupervisorState>>,
    limit: Option<u32>,
) -> Result<Vec<SupervisorAuditRecord>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_audit(&db, limit.unwrap_or(100).clamp(1, 500)).map_err(|error| error.to_string())
}

fn validate_automatic_task(task: &AutomaticTask) -> Result<(), String> {
    if task.name.trim().is_empty() || task.board_id.trim().is_empty() {
        return Err("SUPERVISOR_AUTOMATIC_TASK_NAME_AND_BOARD_REQUIRED".to_owned());
    }
    if !matches!(
        task.trigger_type.as_str(),
        "schedule" | "webhook" | "device" | "manual_api"
    ) {
        return Err("SUPERVISOR_AUTOMATIC_TRIGGER_INVALID".to_owned());
    }
    if !task.trigger.is_object() || !task.task_template.is_object() {
        return Err("SUPERVISOR_AUTOMATIC_TASK_SCHEMA_INVALID".to_owned());
    }
    if task.success_conditions.is_empty() {
        return Err("SUPERVISOR_SUCCESS_CONDITION_REQUIRED".to_owned());
    }
    for condition in &task.success_conditions {
        let kind = condition.get("kind").and_then(Value::as_str);
        let supported = match kind {
            Some("exit_code") => condition.get("equals").and_then(Value::as_i64) == Some(0),
            Some("status") => condition.get("equals").and_then(Value::as_str) == Some("success"),
            _ => false,
        };
        if !supported {
            return Err("SUPERVISOR_SUCCESS_CONDITION_UNSUPPORTED".to_owned());
        }
    }
    if task.policy_id.is_none() {
        return Err("SUPERVISOR_AUTOMATIC_POLICY_REQUIRED".to_owned());
    }
    if super::flow::is_device_flow(task) {
        if task.enabled {
            super::flow::validate(task)?;
        } else {
            super::flow::validate_draft(task)?;
        }
    } else {
        scheduler::template_ids(task)?;
    }
    if task.trigger_type == "device" && !super::flow::is_device_flow(task) {
        let vendor = task
            .trigger
            .get("vendor_id")
            .or_else(|| task.trigger.get("vendorId"))
            .and_then(Value::as_u64);
        let product = task
            .trigger
            .get("product_id")
            .or_else(|| task.trigger.get("productId"))
            .and_then(Value::as_u64);
        let event = task
            .trigger
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("arrived");
        if vendor.is_none_or(|value| value > u64::from(u16::MAX))
            || product.is_none_or(|value| value > u64::from(u16::MAX))
            || !matches!(event, "arrived" | "left")
        {
            return Err("SUPERVISOR_DEVICE_TRIGGER_SCOPE_REQUIRED".to_owned());
        }
    }
    if task.trigger_type == "webhook" && contains_sensitive_key(&task.trigger) {
        return Err("SUPERVISOR_WEBHOOK_SECRET_MUST_USE_KEYRING_FIELD".to_owned());
    }
    Ok(())
}

fn contains_sensitive_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            key.contains("secret")
                || key.contains("token")
                || key.contains("authorization")
                || key.contains("api_key")
                || contains_sensitive_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_key),
        _ => false,
    }
}

#[tauri::command]
pub fn supervisor_list_automatic_tasks(
    state: State<'_, Arc<SupervisorState>>,
) -> Result<Vec<AutomaticTask>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_automatic_tasks(&db).map_err(|error| error.to_string())
}

fn removes_webhook_credential(previous: Option<&AutomaticTask>, candidate: &AutomaticTask) -> bool {
    previous.is_some_and(|task| task.trigger_type == "webhook")
        && candidate.trigger_type != "webhook"
}

#[tauri::command]
pub fn supervisor_save_automatic_task(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
    mut task: AutomaticTask,
    webhook_secret: Option<String>,
) -> Result<AutomaticTask, String> {
    validate_automatic_task(&task)?;
    {
        let workbench_db = workbench.db.lock().map_err(|error| error.to_string())?;
        let mut supervisor_db = state.db.lock().map_err(|error| error.to_string())?;
        storage::sync_board_identities(&mut supervisor_db, &workbench_db)
            .map_err(|error| error.to_string())?;
    }
    let webhook_secret = webhook_secret
        .as_deref()
        .map(str::trim)
        .filter(|secret| !secret.is_empty());
    if task.trigger_type == "webhook"
        && task.enabled
        && webhook_secret.is_none()
        && task
            .id
            .is_none_or(|id| super::credentials::resolve_webhook(id).is_err())
    {
        return Err("SUPERVISOR_WEBHOOK_SECRET_REQUIRED".to_owned());
    }
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let policy = storage::policy(&db, task.policy_id.unwrap_or_default())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_POLICY_NOT_FOUND".to_owned())?;
        if task.enabled && !policy.enabled {
            return Err("SUPERVISOR_POLICY_DISABLED".to_owned());
        }
        let task_scopes = if super::flow::is_device_flow(&task) {
            let scopes = if task.enabled {
                super::flow::validate(&task)?
            } else {
                super::flow::validate_draft(&task)?
            };
            if scopes.is_empty() {
                vec![task.board_id.clone()]
            } else {
                scopes
            }
        } else {
            vec![task.board_id.clone()]
        };
        if task_scopes
            .iter()
            .any(|scope| !policy.board_scope.iter().any(|board| board == scope))
        {
            return Err("SUPERVISOR_AUTOMATIC_POLICY_SCOPE_MISMATCH".to_owned());
        }
    }
    if task.enabled && task.trigger_type == "schedule" && task.next_run_at.is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        task.next_run_at = scheduler::next_schedule(&task, now);
        if task.next_run_at.is_none() {
            return Err("SUPERVISOR_SCHEDULE_INTERVAL_REQUIRED".to_owned());
        }
    }
    let old = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::list_automatic_tasks(&db)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|item| item.id == task.id)
    };
    let fields = crate::config_audit::changed_fields(
        old.as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            .as_ref(),
        serde_json::to_value(&task).ok().as_ref(),
        &[
            "name",
            "enabled",
            "triggerType",
            "trigger",
            "boardId",
            "taskTemplate",
            "policyId",
            "successConditions",
            "nextRunAt",
            "credential",
        ],
    );
    let operation = if task.id.is_some() {
        "update"
    } else {
        "create"
    };
    let removes_webhook = removes_webhook_credential(old.as_ref(), &task);
    let credential_changed = webhook_secret.is_some() || removes_webhook;
    let scope_id = task.id.map_or_else(
        || format!("automatic-task-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("automatic-task:{id}"),
    );
    let audit = crate::config_audit::begin(
        &app,
        "automatic_task",
        scope_id,
        "automatic_task",
        operation,
        fields,
        json!({"enabled": task.enabled, "triggerType": task.trigger_type, "boardScope": task.board_id, "successConditionCount": task.success_conditions.len(), "credentialChanged": credential_changed}),
    )?;
    let previous_webhook = if credential_changed {
        task.id
            .and_then(|id| super::credentials::resolve_webhook(id).ok())
    } else {
        None
    };
    let result = (|| {
        let mut db = state.db.lock().map_err(|error| error.to_string())?;
        let transaction = db.transaction().map_err(|error| error.to_string())?;
        let id =
            storage::save_automatic_task(&transaction, &task).map_err(|error| error.to_string())?;
        if task.trigger_type == "webhook" {
            if let Some(secret) = webhook_secret {
                super::credentials::save_webhook(id, secret)?;
            }
        } else if removes_webhook {
            super::credentials::delete_webhook(id)?;
        }
        if let Err(error) = transaction.commit() {
            let compensated = match (credential_changed, previous_webhook.as_deref()) {
                (true, Some(value)) => super::credentials::save_webhook(id, value),
                (true, None) => super::credentials::delete_webhook(id),
                (false, _) => Ok(()),
            };
            return if compensated.is_ok() {
                Err(error.to_string())
            } else {
                Err("CONFIG_COMPENSATION_FAILED".to_owned())
            };
        }
        storage::list_automatic_tasks(&db)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|item| item.id == Some(id))
            .ok_or_else(|| "SUPERVISOR_AUTOMATIC_TASK_NOT_FOUND_AFTER_SAVE".to_owned())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub fn supervisor_delete_automatic_task(
    app: AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    task_id: i64,
) -> Result<(), String> {
    if task_id <= 0 {
        return Err("SUPERVISOR_AUTOMATIC_TASK_ID_INVALID".to_owned());
    }
    let audit = crate::config_audit::begin(
        &app,
        "automatic_task",
        format!("automatic-task:{task_id}"),
        "automatic_task",
        "delete",
        vec![],
        json!({}),
    )?;
    let had_webhook = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::automatic_task(&db, task_id)
            .map_err(|error| error.to_string())?
            .is_some_and(|task| task.trigger_type == "webhook")
    };
    let previous_webhook = if had_webhook {
        super::credentials::resolve_webhook(task_id).ok()
    } else {
        None
    };
    let result = (|| {
        if had_webhook {
            super::credentials::delete_webhook(task_id)?;
        }
        let db = state.db.lock().map_err(|error| error.to_string())?;
        if let Err(error) = storage::delete_automatic_task(&db, task_id) {
            return match (had_webhook, previous_webhook.as_deref()) {
                (true, Some(value))
                    if super::credentials::save_webhook(task_id, value).is_err() =>
                {
                    Err("CONFIG_COMPENSATION_FAILED".to_owned())
                }
                _ => Err(error.to_string()),
            };
        }
        Ok(())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

fn validate_device_flow_credential_reference(reference: &str) -> Result<&str, String> {
    let reference = reference.trim();
    if reference.is_empty()
        || reference.len() > 160
        || !reference
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ":._-".contains(character))
    {
        return Err("SUPERVISOR_DEVICE_FLOW_CREDENTIAL_REFERENCE_INVALID".to_owned());
    }
    Ok(reference)
}

#[tauri::command]
pub fn supervisor_save_device_flow_credential(
    app: AppHandle,
    reference: String,
    password: Option<String>,
) -> Result<(), String> {
    let reference = validate_device_flow_credential_reference(&reference)?;
    let entry = keyring::Entry::new(&crate::keyring_service("dshanpi-lynx-device-flow"), reference)
        .map_err(|_| "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_UNAVAILABLE".to_owned())?;
    let operation = if password.as_ref().is_some_and(|value| !value.is_empty()) {
        "update"
    } else {
        "delete"
    };
    let reference_digest = crate::content_digest::digest(&app, &[reference.as_bytes()])?;
    let audit = crate::config_audit::begin(
        &app,
        "device_flow_credential",
        format!("credential-reference:{reference_digest}"),
        "device_flow_credential",
        operation,
        vec!["credential".to_owned()],
        json!({"credentialChanged": true}),
    )?;
    let result = match password.filter(|value| !value.is_empty()) {
        Some(password) => entry
            .set_password(&password)
            .map_err(|_| "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_SAVE_FAILED".to_owned()),
        None => entry
            .delete_credential()
            .map_err(|_| "SUPERVISOR_DEVICE_FLOW_CREDENTIAL_DELETE_FAILED".to_owned()),
    };
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[tauri::command]
pub async fn supervisor_analyze_failure(
    app: tauri::AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    context: FailureContext,
) -> Result<SupervisorAnalysisResult, String> {
    engine::analyze(Some(&app), state.inner(), context).await
}

#[tauri::command]
pub fn supervisor_list_runs(
    state: State<'_, Arc<SupervisorState>>,
    limit: Option<u32>,
) -> Result<Vec<SupervisorRun>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_runs(&db, limit.unwrap_or(100).clamp(1, 500)).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn supervisor_list_rounds(
    state: State<'_, Arc<SupervisorState>>,
    run_id: String,
    limit: Option<u32>,
) -> Result<Vec<super::types::SupervisorRound>, String> {
    if run_id.trim().is_empty() {
        return Err("SUPERVISOR_RUN_ID_REQUIRED".to_owned());
    }
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_rounds(&db, &run_id, limit.unwrap_or(100).clamp(1, 500))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn supervisor_control_run(
    state: State<'_, Arc<SupervisorState>>,
    run_id: String,
    action: String,
) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err("SUPERVISOR_RUN_ID_REQUIRED".to_owned());
    }
    let (status, reason) = match action.as_str() {
        "pause" => (
            "paused",
            "用户暂停 AI 监督；当前已授权硬件步骤安全结束后停止",
        ),
        "resume" => ("running", "用户恢复 AI 监督"),
        "takeover" => ("manual", "用户接管并转为手动模式"),
        "terminate" => ("terminated", "用户终止监督；不强制中断正在执行的硬件步骤"),
        _ => return Err("SUPERVISOR_RUN_CONTROL_INVALID".to_owned()),
    };
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let changed =
        storage::set_run_status(&db, &run_id, status, reason).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("SUPERVISOR_RUN_NOT_FOUND".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub fn supervisor_list_human_requests(
    state: State<'_, Arc<SupervisorState>>,
    waiting_only: Option<bool>,
) -> Result<Vec<HumanInterventionRequest>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    storage::list_human_requests(&db, waiting_only.unwrap_or(false))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn supervisor_resolve_human_request(
    app: tauri::AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    request_id: i64,
    resolution: String,
) -> Result<(), String> {
    let status = match resolution.as_str() {
        "continue" => "running",
        "manual" => "manual",
        "terminate" => "terminated",
        _ => return Err("SUPERVISOR_HUMAN_RESOLUTION_INVALID".to_owned()),
    };
    let (run_id, task_id, board_id) = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let run_id = storage::resolve_human_request(&db, request_id, &resolution)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_HUMAN_REQUEST_NOT_FOUND".to_owned())?;
        storage::set_run_status(&db, &run_id, status, "人工介入已处理")
            .map_err(|error| error.to_string())?;
        let (task_id, board_id, _) = storage::run_scope(&db, &run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?;
        (run_id, task_id, board_id)
    };
    super::audit_human_resolution(
        &app,
        &run_id,
        &task_id,
        &board_id,
        request_id,
        &resolution,
        status,
    );
    Ok(())
}

#[tauri::command]
pub fn supervisor_execute_action(
    app: tauri::AppHandle,
    state: State<'_, Arc<SupervisorState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
    run_id: String,
    round_index: u32,
    approved: Option<bool>,
) -> Result<ActionExecutionResult, String> {
    if run_id.trim().is_empty() || round_index == 0 {
        return Err("SUPERVISOR_EXECUTION_SCOPE_INVALID".to_owned());
    }
    let result = executor::execute(
        &app,
        state.inner(),
        workbench.inner(),
        &run_id,
        round_index,
        approved.unwrap_or(false),
    )?;
    if approved.unwrap_or(false) {
        let (task_id, board_id, _) = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::run_scope(&db, &run_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SUPERVISOR_RUN_NOT_FOUND".to_owned())?
        };
        super::audit_action_approval(
            &app,
            &run_id,
            &task_id,
            &board_id,
            round_index,
            &result.action,
        );
    }
    Ok(result)
}

#[tauri::command]
pub fn supervisor_trigger_automatic_task(
    app: tauri::AppHandle,
    task_id: i64,
    operator_approved: Option<bool>,
) -> Result<String, String> {
    if task_id <= 0 {
        return Err("SUPERVISOR_AUTOMATIC_TASK_ID_INVALID".to_owned());
    }
    scheduler::trigger(&app, task_id, false, operator_approved.unwrap_or(false))
}

pub(crate) fn resolve_default_provider(
    state: &SupervisorState,
) -> Result<(AiProviderConfig, String), String> {
    let provider = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let id = storage::default_provider_id(&db)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "AI_PROVIDER_NOT_CONFIGURED".to_owned())?;
        storage::provider(&db, id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "AI_PROVIDER_NOT_FOUND".to_owned())?
    };
    let key = credentials::resolve(provider.id.unwrap_or_default())?;
    Ok((provider, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_flow_credential_reference_is_bounded_and_path_safe() {
        assert_eq!(
            validate_device_flow_credential_reference("serial-login:linux:4:node-1").unwrap(),
            "serial-login:linux:4:node-1"
        );
        assert!(validate_device_flow_credential_reference("").is_err());
        assert!(validate_device_flow_credential_reference("../../password").is_err());
        assert!(validate_device_flow_credential_reference(&"a".repeat(161)).is_err());
    }

    #[tokio::test]
    #[ignore = "requires the live supervisor database, keyring and provider network"]
    async fn live_provider_test_button_path() {
        let database = std::env::var("LYNX_LIVE_SUPERVISOR_DB")
            .expect("LYNX_LIVE_SUPERVISOR_DB must point to the live test database");
        let provider_id = std::env::var("LYNX_LIVE_AI_PROVIDER_ID")
            .unwrap_or_else(|_| "1".to_owned())
            .parse::<i64>()
            .expect("provider id must be an integer");
        let state = SupervisorState::open(std::path::Path::new(&database))
            .expect("live supervisor database must open");
        let result = test_provider_core(&state, provider_id, "live-provider-test")
            .await
            .expect("provider test command path should complete");
        assert!(result.ok, "{}: {}", result.status, result.message);
        assert_eq!(result.status, "available");
    }

    #[test]
    fn public_http_provider_is_rejected() {
        assert!(validate_base_url("http://api.deepseek.com/v1").is_err());
        assert!(validate_base_url("https://api.deepseek.com/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:11434/v1").is_ok());
    }

    #[test]
    fn endpoint_preserves_compatible_base_path() {
        assert_eq!(
            endpoint("https://api.openai.com/v1/", "models").unwrap(),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn policy_rejects_overlapping_permissions() {
        let policy = SupervisorPolicy {
            id: None,
            name: "default".to_owned(),
            enabled: true,
            provider_id: None,
            max_attempts: 6,
            max_duration_seconds: 1800,
            max_same_error: 2,
            max_power_cycles: 3,
            max_flash_retries: 2,
            wait_device_seconds: 30,
            cost_limit: Some(1.0),
            allowed_actions: vec!["power_cycle".to_owned()],
            approval_actions: vec!["power_cycle".to_owned()],
            board_scope: vec![],
            success_summary: true,
            notification_mode: "desktop".to_owned(),
        };
        assert!(validate_policy(&policy).is_err());
    }

    #[test]
    fn enabled_policies_cannot_claim_the_same_board() {
        let existing = SupervisorPolicy {
            id: Some(1),
            name: "existing".to_owned(),
            enabled: true,
            provider_id: None,
            max_attempts: 1,
            max_duration_seconds: 30,
            max_same_error: 1,
            max_power_cycles: 0,
            max_flash_retries: 0,
            wait_device_seconds: 10,
            cost_limit: None,
            allowed_actions: vec!["rescan_usb".to_owned()],
            approval_actions: vec![],
            board_scope: vec!["linux:4".to_owned()],
            success_summary: true,
            notification_mode: "desktop".to_owned(),
        };
        let mut candidate = existing.clone();
        candidate.id = Some(2);
        assert_eq!(
            conflicting_policy_board(&candidate, std::slice::from_ref(&existing)),
            Some("linux:4".to_owned())
        );
        candidate.enabled = false;
        assert_eq!(
            conflicting_policy_board(&candidate, std::slice::from_ref(&existing)),
            None
        );
    }

    #[test]
    fn automatic_task_requires_fixed_board_and_verification() {
        let mut task = AutomaticTask {
            id: None,
            name: "nightly".to_owned(),
            enabled: false,
            trigger_type: "schedule".to_owned(),
            trigger: serde_json::json!({"hour": 2}),
            board_id: "linux:1".to_owned(),
            task_template: serde_json::json!({"kind":"workflow","project_id":1,"device_id":1}),
            policy_id: None,
            success_conditions: vec![],
            next_run_at: None,
            last_run_at: None,
        };
        assert!(validate_automatic_task(&task).is_err());
        task.success_conditions = vec![serde_json::json!({"kind": "exit_code", "equals": 0})];
        task.policy_id = Some(1);
        assert!(validate_automatic_task(&task).is_ok());
        task.board_id.clear();
        assert!(validate_automatic_task(&task).is_err());
    }

    #[test]
    fn non_webhook_tasks_do_not_require_platform_secure_storage() {
        let task = AutomaticTask {
            id: None,
            name: "manual".to_owned(),
            enabled: true,
            trigger_type: "manual_api".to_owned(),
            trigger: serde_json::json!({}),
            board_id: "linux:1".to_owned(),
            task_template: serde_json::json!({}),
            policy_id: Some(1),
            success_conditions: vec![],
            next_run_at: None,
            last_run_at: None,
        };
        assert!(!removes_webhook_credential(None, &task));

        let mut previous = task.clone();
        previous.trigger_type = "webhook".to_owned();
        assert!(removes_webhook_credential(Some(&previous), &task));

        let mut webhook_update = previous.clone();
        webhook_update.name = "updated webhook".to_owned();
        assert!(!removes_webhook_credential(
            Some(&previous),
            &webhook_update
        ));
    }

    #[test]
    fn incomplete_device_flow_can_only_be_saved_as_a_disabled_draft() {
        let mut task = AutomaticTask {
            id: None,
            name: "work in progress".to_owned(),
            enabled: false,
            trigger_type: "manual_api".to_owned(),
            trigger: serde_json::json!({}),
            board_id: "linux:4".to_owned(),
            task_template: serde_json::json!({
                "kind":"device_flow", "version":1, "nodes":[], "edges":[]
            }),
            policy_id: Some(1),
            success_conditions: vec![serde_json::json!({"kind":"exit_code","equals":0})],
            next_run_at: None,
            last_run_at: None,
        };
        assert!(validate_automatic_task(&task).is_ok());
        task.enabled = true;
        assert_eq!(
            validate_automatic_task(&task).unwrap_err(),
            "SUPERVISOR_DEVICE_FLOW_VERSION_OR_NODES_INVALID"
        );
    }
}

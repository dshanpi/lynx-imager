use serde_json::{json, Value};

pub struct ConfigAuditContext {
    pub intent_id: String,
    pub correlation_id: String,
    pub scope_type: &'static str,
    pub scope_id: String,
    pub resource_type: &'static str,
    pub operation: &'static str,
    pub changed_fields: Vec<String>,
    pub safe_summary: Value,
}

pub fn changed_fields(old: Option<&Value>, new: Option<&Value>, allowed: &[&str]) -> Vec<String> {
    allowed
        .iter()
        .filter(|field| old.and_then(|v| v.get(**field)) != new.and_then(|v| v.get(**field)))
        .map(|field| (*field).to_owned())
        .collect()
}

pub fn begin(
    app: &tauri::AppHandle,
    scope_type: &'static str,
    scope_id: String,
    resource_type: &'static str,
    operation: &'static str,
    changed_fields: Vec<String>,
    safe_summary: Value,
) -> Result<ConfigAuditContext, String> {
    begin_linked(
        app,
        scope_type,
        scope_id,
        resource_type,
        operation,
        changed_fields,
        safe_summary,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn begin_linked(
    app: &tauri::AppHandle,
    scope_type: &'static str,
    scope_id: String,
    resource_type: &'static str,
    operation: &'static str,
    changed_fields: Vec<String>,
    safe_summary: Value,
    correlation_id: Option<String>,
) -> Result<ConfigAuditContext, String> {
    let correlation_id = correlation_id
        .unwrap_or_else(|| format!("config-{}-{:016x}", now_ms(), rand::random::<u64>()));
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "configuration".to_owned(),
            action: format!("{resource_type}_{operation}_intended"),
            scope_type: scope_type.to_owned(),
            scope_id: scope_id.clone(),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(correlation_id.clone()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备修改持久配置".to_owned(),
            metadata: json!({
                "resourceType": resource_type,
                "operation": operation,
                "changedFields": changed_fields,
                "safeSummary": safe_summary,
            }),
        },
    )?;
    let crate::audit::AuditWriteOutcome::Recorded { event_id } = outcome else {
        return Err("AUDIT_REQUIRED_DEGRADED".to_owned());
    };
    Ok(ConfigAuditContext {
        intent_id: event_id,
        correlation_id,
        scope_type,
        scope_id,
        resource_type,
        operation,
        changed_fields,
        safe_summary,
    })
}

pub fn finish<T>(app: &tauri::AppHandle, context: &ConfigAuditContext, result: &Result<T, String>) {
    let (status, error_code, summary) = match result {
        Ok(_) if context.changed_fields.is_empty() => ("unchanged", None, "持久配置无需变更"),
        Ok(_) => ("success", None, "持久配置已修改"),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "CONFIG_CHANGE_FAILED",
            )),
            "持久配置修改失败",
        ),
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "configuration".to_owned(),
            action: format!("{}_{}_completed", context.resource_type, context.operation),
            scope_type: context.scope_type.to_owned(),
            scope_id: context.scope_id.clone(),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(context.correlation_id.clone()),
            causation_id: Some(context.intent_id.clone()),
            result: status.to_owned(),
            error_code,
            summary: summary.to_owned(),
            metadata: json!({
                "resourceType": context.resource_type,
                "operation": context.operation,
                "changedFields": context.changed_fields,
                "safeSummary": context.safe_summary,
            }),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_exposes_only_allowlisted_field_names() {
        let old = json!({"enabled": false, "address": "old"});
        let new = json!({"enabled": true, "address": "new"});
        assert_eq!(
            changed_fields(Some(&old), Some(&new), &["enabled"]),
            ["enabled"]
        );
    }
}

//! Policy-bound tutoring over verified course snapshots. No executor.
use super::{courses, error, manifest, LearnError};
use crate::{auth::state::AuthState, supervisor::SupervisorState};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

mod engineering;
mod redaction;
pub(super) use redaction::clean as redact_context;
pub(crate) use redaction::task_log as redact_task_log;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TutorRequest {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub unit_id: String,
    pub question: String,
    pub hint_level: String,
    #[serde(default)]
    pub engineering: Option<engineering::EngineeringRequest>,
}
fn tutor_context(
    course: &manifest::VerifiedCourse,
    content: &courses::LearnUnitContent,
    question: &str,
    level: &str,
) -> Result<serde_json::Value, LearnError> {
    let policy = &course.agent_policy;
    if !policy.enabled || !policy.hint_levels.iter().any(|v| v == level) {
        return Err(error(
            "LEARN_TUTOR_DISABLED",
            "课程未开放此级别的 AI 指导",
            false,
        ));
    }
    if question.trim().is_empty() || question.chars().count() > 1000 {
        return Err(error(
            "LEARN_TUTOR_QUESTION_INVALID",
            "请输入 1 到 1000 字的问题",
            false,
        ));
    }
    let unit = course
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .find(|u| u.unit_id == content.unit_id)
        .ok_or_else(super::schema_error)?;
    let documents:Vec<_>=content.documents.iter().filter(|d|policy.context_resource_refs.contains(&d.resource_id)).map(|d|serde_json::json!({"resourceId":d.resource_id,"sha256":d.sha256,"markdown":redact_context(&d.markdown)})).collect();
    let value = serde_json::json!({"courseId":course.manifest.course_id,"releaseId":course.manifest.release_id,"unitId":unit.unit_id,"title":unit.title,"objective":unit.objective,"documents":documents,"hintLevel":level,"question":redact_context(question),"executedAction":false});
    context_limit(&value, policy)?;
    Ok(value)
}
fn context_limit(
    value: &serde_json::Value,
    policy: &manifest::AgentPolicy,
) -> Result<(), LearnError> {
    if value.to_string().len() > policy.max_context_bytes.min(65536) as usize {
        return Err(error(
            "LEARN_TUTOR_CONTEXT_LIMIT",
            "当前教材超过课程 AI 上下文限制，请联系维护者调整课节或策略",
            false,
        ));
    }
    Ok(())
}
#[tauri::command]
pub async fn learn_ask_tutor(
    app: AppHandle,
    auth: State<'_, Arc<AuthState>>,
    supervisor: State<'_, Arc<SupervisorState>>,
    local: State<'_, Arc<crate::learning::local::LocalLearningState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
    request: TutorRequest,
) -> Result<crate::learning::commands::LearningTutorAnswer, String> {
    let user = auth
        .get_user()
        .await?
        .ok_or("LEARN_AUTH_REQUIRED:请先登录")?
        .id;
    let result = async {
        let (base, token) = courses::context(&auth).await?;
        let keys = manifest::official_keys()?;
        let course = courses::course(
            &base,
            &token,
            &request.enrollment_id,
            &request.release_id,
            &request.manifest_sha256,
            &keys,
        )
        .await?;
        let content: courses::LearnUnitContent = courses::read(
            &base,
            &token,
            &[
                "enrollments",
                &request.enrollment_id,
                "units",
                &request.unit_id,
            ],
        )
        .await?;
        courses::validate_documents(&content, &course, &request.unit_id)?;
        let context = tutor_context(&course, &content, &request.question, &request.hint_level)?;
        Ok((course, context))
    }
    .await;
    let (course, mut context) =
        result.map_err(|e: LearnError| format!("{}:{}", e.machine_code, e.message))?;
    let mut scope = vec![
        "verified_course_policy".into(),
        "signed_document_snapshots".into(),
        "redacted_question".into(),
    ];
    if auth.get_user().await?.map(|u| u.id) != Some(user) {
        return Err("LEARN_ACCOUNT_CHANGED:账号已切换，请重新提问".into());
    }
    if let Some(selection) = &request.engineering {
        context["engineering"] = engineering::collect(
            &auth, &local, &workbench, user, &course, &request, selection,
        )
        .await?;
        context_limit(&context, &course.agent_policy)
            .map_err(|e| format!("{}:{}", e.machine_code, e.message))?;
        scope.push("opted_in_source_diff_and_local_practice".into());
    }
    let answer = crate::learning::commands::ask_verified_tutor_context(
        app,
        supervisor.inner().as_ref(),
        &request.enrollment_id,
        &request.unit_id,
        context,
        scope,
    )
    .await?;
    if auth.get_user().await?.map(|u| u.id) != Some(user) {
        return Err("LEARN_ACCOUNT_CHANGED:账号已切换，本次回答不再显示".into());
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::v2::manifest::tests::verified;
    #[test]
    fn explicit_engineering_context_request_is_supported_without_accepting_forged_results() {
        let request = serde_json::json!({"enrollmentId":"enr","releaseId":"rel",
            "manifestSha256":"a".repeat(64),"unitId":"unit","question":"why?",
            "hintLevel":"hint","engineering":{"activityId":"build","sourcePaths":["src/main.c"]}});
        assert!(serde_json::from_value::<TutorRequest>(request.clone()).is_ok());
        let mut forged = request;
        forged["engineering"]["result"] = serde_json::json!({"status":"PASS"});
        assert!(serde_json::from_value::<TutorRequest>(forged).is_err());
    }
    #[test]
    fn tutor_policy_gates_context_and_never_exposes_unreferenced_material() {
        let mut course = verified();
        let content = courses::LearnUnitContent {
            enrollment_id: course.enrollment_id.clone(),
            release_id: course.manifest.release_id.clone(),
            unit_id: course.manifest.chapters[0].units[0].unit_id.clone(),
            documents: vec![],
        };
        course.agent_policy.enabled = false;
        assert!(tutor_context(&course, &content, "解释这一步", "explain").is_err());
        course.agent_policy.enabled = true;
        course.agent_policy.hint_levels = vec!["explain".into()];
        course.agent_policy.max_context_bytes = 8192;
        assert!(tutor_context(&course, &content, "给出答案", "solution").is_err());
        assert!(tutor_context(&course, &content, " ", "explain").is_err());
        let value = tutor_context(&course, &content, "解释这一步", "explain").unwrap();
        assert_eq!(value["executedAction"], false);
        assert!(value.get("tools").is_none());
        let mut oversized = value;
        oversized["engineering"] = serde_json::json!({"diff":"x".repeat(8192)});
        assert!(context_limit(&oversized, &course.agent_policy).is_err());
    }
}

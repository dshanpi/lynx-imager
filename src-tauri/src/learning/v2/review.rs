//! Shared explicit assessment and report clients. No local grading or execution.
use super::execution::sync::review_gate;
use super::{courses, error, manifest, schema_error, LearnError};
use crate::{auth::state::AuthState, learning::local::LocalLearningState};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tauri::State;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pin {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitOutcome {
    pub chapter_id: String,
    pub unit_id: String,
    pub title: String,
    pub objective: String,
    pub required: bool,
    pub passed: bool,
    pub state: String,
    pub current_attempt_id: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub revision: i64,
    pub all_required_passed: bool,
    pub units: Vec<UnitOutcome>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub status: String,
    pub passed_rules: Vec<String>,
    pub failed_rules: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub unverified: Vec<String>,
    pub verification_level: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResult {
    pub attempt_id: String,
    pub assessment: Assessment,
    pub summary: Summary,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewRequest {
    #[serde(flatten)]
    pub pin: Pin,
    pub expected_revision: i64,
    pub unit_id: String,
    #[serde(default)]
    pub attempt_id: String,
    pub answers: HashMap<String, String>,
    #[serde(default)]
    pub reflection: String,
    pub request_id: String,
}
#[derive(Deserialize)]
struct Data<T> {
    data: T,
}
fn review_error(e: LearnError) -> LearnError {
    if e.machine_code == "LEARN_PREVIEW_STALE" {
        error(
            "LEARN_REVIEW_CONFLICT",
            "评测输入或记录已更新，请同步后再提交",
            false,
        )
    } else {
        e
    }
}
async fn context(
    auth: &AuthState,
    p: &Pin,
) -> Result<(String, String, manifest::VerifiedCourse), LearnError> {
    let (base, token) = courses::context(auth).await?;
    let c = courses::course(
        &base,
        &token,
        &p.enrollment_id,
        &p.release_id,
        &p.manifest_sha256,
        &manifest::official_keys()?,
    )
    .await?;
    Ok((base, token, c))
}
fn validate_summary(s: &Summary, c: &manifest::VerifiedCourse) -> Result<(), LearnError> {
    if s.enrollment_id != c.enrollment_id
        || s.release_id != c.manifest.release_id
        || s.manifest_sha256 != c.manifest_sha256
        || s.revision < 1
        || s.units.len()
            != c.manifest
                .chapters
                .iter()
                .map(|c| c.units.len())
                .sum::<usize>()
    {
        return Err(schema_error());
    }
    let mut seen = HashSet::new();
    for u in &s.units {
        let expected = c
            .manifest
            .chapters
            .iter()
            .find(|ch| ch.chapter_id == u.chapter_id)
            .and_then(|ch| ch.units.iter().find(|v| v.unit_id == u.unit_id))
            .ok_or_else(schema_error)?;
        if !seen.insert(&u.unit_id)
            || u.title != expected.title
            || u.objective != expected.objective
            || u.required != expected.activities.iter().any(|a| a.required)
            || u.passed != (u.state == "passed")
            || ![
                "not_started",
                "created",
                "running",
                "needs_evidence",
                "passed",
                "failed",
                "cancelled",
            ]
            .contains(&u.state.as_str())
            || (!u.current_attempt_id.is_empty() && !manifest::valid_id(&u.current_attempt_id))
        {
            return Err(schema_error());
        }
    }
    if s.all_required_passed != s.units.iter().filter(|u| u.required).all(|u| u.passed) {
        return Err(schema_error());
    }
    Ok(())
}
#[tauri::command]
pub async fn learn_get_learning_summary(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    request: Pin,
) -> Result<Summary, LearnError> {
    let owner = review_gate::owner(&auth).await?;
    let (base, token, c) = context(&auth, &request).await?;
    let summary: Summary = courses::read(
        &base,
        &token,
        &["enrollments", &request.enrollment_id, "learning-summary"],
    )
    .await
    .map_err(review_error)?;
    validate_summary(&summary, &c)?;
    if review_gate::owner(&auth).await? != owner {
        return Err(review_gate::fault("LEARN_ACCOUNT_CHANGED".into()));
    }
    {
        let db = local
            .db
            .lock()
            .map_err(|_| review_gate::fault("LEARN_STORE_UNAVAILABLE".into()))?;
        for unit in &summary.units {
            if ["passed", "failed", "cancelled"].contains(&unit.state.as_str())
                && !unit.current_attempt_id.is_empty()
            {
                review_gate::reviewed(&db, owner, &request, &unit.current_attempt_id)
                    .map_err(review_gate::fault)?;
            }
        }
    }
    Ok(summary)
}
#[tauri::command]
pub async fn learn_submit_review(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<crate::workbench::WorkbenchState>>,
    request: ReviewRequest,
) -> Result<ReviewResult, LearnError> {
    let owner = review_gate::owner(&auth).await?;
    let _write = review_gate::WRITE_GATE.lock().await;
    if review_gate::owner(&auth).await? != owner {
        return Err(review_gate::fault("LEARN_ACCOUNT_CHANGED".into()));
    }
    let (base, token, c) = context(&auth, &request.pin).await?;
    if request.expected_revision < 1
        || !manifest::valid_id(&request.request_id)
        || request.reflection.len() > 4096
        || request.answers.len() > 100
        || (!request.attempt_id.is_empty() && !manifest::valid_id(&request.attempt_id))
    {
        return Err(schema_error());
    }
    let unit = c
        .manifest
        .chapters
        .iter()
        .flat_map(|ch| &ch.units)
        .find(|u| u.unit_id == request.unit_id)
        .ok_or_else(schema_error)?;
    for (id, value) in &request.answers {
        if value.len() > 4096
            || value.trim().is_empty()
            || value.contains('\0')
            || !unit.activities.iter().any(|a| {
                a.activity_id == *id && a.completion_mode == "quiz" && a.action_refs.is_empty()
            })
        {
            return Err(schema_error());
        }
    }
    let body = serde_json::json!({"releaseId":request.pin.release_id,"manifestSha256":request.pin.manifest_sha256,"expectedRevision":request.expected_revision,"unitId":request.unit_id,"attemptId":request.attempt_id,"answers":request.answers,"reflection":request.reflection});
    if review_gate::owner(&auth).await? != owner {
        return Err(review_gate::fault("LEARN_ACCOUNT_CHANGED".into()));
    }
    let practice = {
        let db = local
            .db
            .lock()
            .map_err(|_| review_gate::fault("LEARN_STORE_UNAVAILABLE".into()))?;
        review_gate::review_plan(&db, owner, &request, &body).map_err(review_gate::fault)?
    };
    if let Some(batch_id) = practice {
        let observed = super::execution::sync::freshness::check(
            &auth, &local, &workbench, owner, &batch_id, &c,
        )
        .await
        .map_err(|code| {
            review_gate::fault(if code == "LEARN_ACCOUNT_CHANGED" {
                code
            } else {
                "LEARN_EVIDENCE_CHECK_UNAVAILABLE".into()
            })
        })?;
        if observed.status != "current" {
            return Err(review_gate::fault("LEARN_EVIDENCE_STALE".into()));
        }
    }
    {
        let mut db = local
            .db
            .lock()
            .map_err(|_| review_gate::fault("LEARN_STORE_UNAVAILABLE".into()))?;
        review_gate::authorize(&mut db, owner, &request, &body).map_err(review_gate::fault)?;
    }
    let raw = super::post_response(
        courses::url(
            &base,
            &["enrollments", &request.pin.enrollment_id, "reviews"],
        )?,
        &token,
        &body,
        &request.request_id,
        4 * 1024 * 1024,
    )
    .await
    .map_err(review_error)?;
    let value: Data<ReviewResult> = serde_json::from_slice(&raw).map_err(|_| schema_error())?;
    validate_summary(&value.data.summary, &c)?;
    if !manifest::valid_id(&value.data.attempt_id)
        || (!request.attempt_id.is_empty() && request.attempt_id != value.data.attempt_id)
        || !["passed", "failed", "needs_evidence"].contains(&value.data.assessment.status.as_str())
    {
        return Err(schema_error());
    }
    if review_gate::owner(&auth).await? != owner {
        return Err(review_gate::fault("LEARN_ACCOUNT_CHANGED".into()));
    }
    {
        let db = local
            .db
            .lock()
            .map_err(|_| review_gate::fault("LEARN_STORE_UNAVAILABLE".into()))?;
        review_gate::reviewed(&db, owner, &request.pin, &value.data.attempt_id)
            .map_err(review_gate::fault)?;
    }
    Ok(value.data)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportRequest {
    #[serde(flatten)]
    pub pin: Pin,
    pub scope: String,
    #[serde(default)]
    pub chapter_id: String,
    pub input_revision: i64,
    pub request_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportQueued {
    pub job_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportJob {
    pub job_id: String,
    pub status: String,
    pub report: Option<CourseReport>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reflection {
    pub attempt_id: String,
    pub unit_id: String,
    pub activity_id: String,
    pub text: String,
    pub occurred_at: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseReport {
    pub report_id: String,
    pub enrollment_id: String,
    pub release_id: String,
    pub input_revision: i64,
    pub input_digest: String,
    pub scope: String,
    pub chapter_id: String,
    pub status: String,
    pub all_required_passed: bool,
    pub units: Vec<UnitOutcome>,
    pub unverified: Vec<String>,
    #[serde(default)]
    pub reflections: Vec<Reflection>,
    #[serde(flatten)]
    pub details: HashMap<String, serde_json::Value>,
}
fn validate_report(
    r: &CourseReport,
    c: &manifest::VerifiedCourse,
    id: &str,
) -> Result<(), LearnError> {
    if r.report_id != id
        || !manifest::valid_id(id)
        || r.enrollment_id != c.enrollment_id
        || r.release_id != c.manifest.release_id
        || r.input_revision < 1
        || !manifest::valid_digest(&r.input_digest)
        || r.status != "completed"
        || !["chapter", "course"].contains(&r.scope.as_str())
    {
        return Err(schema_error());
    }
    let mut selected = c.manifest.clone();
    if r.scope == "chapter" {
        selected.chapters.retain(|ch| ch.chapter_id == r.chapter_id);
        if selected.chapters.len() != 1 {
            return Err(schema_error());
        }
    } else if !r.chapter_id.is_empty() {
        return Err(schema_error());
    }
    let view = manifest::VerifiedCourse {
        enrollment_id: c.enrollment_id.clone(),
        manifest_sha256: c.manifest_sha256.clone(),
        signing_key_id: c.signing_key_id.clone(),
        manifest: selected,
        missing_capabilities: vec![],
        execution_enabled: false,
        agent_policy: c.agent_policy.clone(),
    };
    let summary = Summary {
        enrollment_id: r.enrollment_id.clone(),
        release_id: r.release_id.clone(),
        manifest_sha256: c.manifest_sha256.clone(),
        revision: r.input_revision,
        all_required_passed: r.all_required_passed,
        units: r.units.clone(),
    };
    validate_summary(&summary, &view)?;
    for note in &r.reflections {
        if !manifest::valid_id(&note.attempt_id)
            || note.text.len() > 8192
            || chrono::DateTime::parse_from_rfc3339(&note.occurred_at).is_err()
            || !view
                .manifest
                .chapters
                .iter()
                .flat_map(|ch| &ch.units)
                .any(|u| {
                    u.unit_id == note.unit_id
                        && u.activities
                            .iter()
                            .any(|a| a.activity_id == note.activity_id)
                })
        {
            return Err(schema_error());
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn learn_request_report(
    auth: State<'_, Arc<AuthState>>,
    request: ReportRequest,
) -> Result<ReportQueued, LearnError> {
    let (base, token, c) = context(&auth, &request.pin).await?;
    if !["chapter", "course"].contains(&request.scope.as_str())
        || request.input_revision < 1
        || !manifest::valid_id(&request.request_id)
        || (request.scope == "chapter"
            && !c
                .manifest
                .chapters
                .iter()
                .any(|ch| ch.chapter_id == request.chapter_id))
    {
        return Err(schema_error());
    }
    let body = serde_json::json!({"scope":request.scope,"chapterId":request.chapter_id,"inputRevision":request.input_revision});
    let raw = super::post_response(
        courses::url(
            &base,
            &["enrollments", &request.pin.enrollment_id, "reports"],
        )?,
        &token,
        &body,
        &request.request_id,
        64 * 1024,
    )
    .await
    .map_err(review_error)?;
    let value: Data<ReportQueued> = serde_json::from_slice(&raw).map_err(|_| schema_error())?;
    if !manifest::valid_id(&value.data.job_id) {
        return Err(schema_error());
    }
    Ok(value.data)
}
#[tauri::command]
pub async fn learn_get_report_job(
    auth: State<'_, Arc<AuthState>>,
    request: Pin,
    job_id: String,
) -> Result<ReportJob, LearnError> {
    let (base, token, c) = context(&auth, &request).await?;
    let job: serde_json::Value = courses::read(&base, &token, &["jobs", &job_id]).await?;
    if job["type"] != "report" || job["jobId"] != job_id {
        return Err(schema_error());
    }
    let status = job["status"].as_str().ok_or_else(schema_error)?.to_string();
    if !["queued", "running", "succeeded", "failed", "cancelled"].contains(&status.as_str()) {
        return Err(schema_error());
    }
    let report = if status == "succeeded" {
        let id = job["result"]["reportId"]
            .as_str()
            .ok_or_else(schema_error)?;
        let value: CourseReport = courses::read(&base, &token, &["reports", id]).await?;
        validate_report(&value, &c, id)?;
        Some(value)
    } else {
        None
    };
    Ok(ReportJob {
        job_id,
        status,
        report,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_request_decodes_wire_and_rejects_unknown_execution() {
        let mut raw = serde_json::json!({"enrollmentId":"e","releaseId":"r","manifestSha256":"a".repeat(64),"expectedRevision":1,"unitId":"u","answers":{},"requestId":"request"});
        assert!(serde_json::from_value::<ReviewRequest>(raw.clone()).is_ok());
        raw["execute"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReviewRequest>(raw).is_err());
    }
    #[test]
    fn report_binds_release_scope_and_required_result() {
        let c = manifest::tests::verified();
        let ch = &c.manifest.chapters[0];
        let u = &ch.units[0];
        let mut r:CourseReport=serde_json::from_value(serde_json::json!({"reportId":"report1","enrollmentId":c.enrollment_id,"releaseId":c.manifest.release_id,"inputRevision":1,"inputDigest":"a".repeat(64),"scope":"chapter","chapterId":ch.chapter_id,"status":"completed","allRequiredPassed":false,"units":[{"chapterId":ch.chapter_id,"unitId":u.unit_id,"title":u.title,"objective":u.objective,"required":true,"passed":false,"state":"not_started","currentAttemptId":""}],"unverified":[],"reflections":[]})).unwrap();
        validate_report(&r, &c, "report1").unwrap();
        r.all_required_passed = true;
        assert!(validate_report(&r, &c, "report1").is_err());
        r.all_required_passed = false;
        r.chapter_id = "other".into();
        assert!(validate_report(&r, &c, "report1").is_err());
    }
    #[test]
    fn summary_rejects_wrong_account_version_activity_and_aggregate() {
        let c = manifest::tests::verified();
        let ch = &c.manifest.chapters[0];
        let u = &ch.units[0];
        let mut s = Summary {
            enrollment_id: c.enrollment_id.clone(),
            release_id: c.manifest.release_id.clone(),
            manifest_sha256: c.manifest_sha256.clone(),
            revision: 1,
            all_required_passed: false,
            units: vec![UnitOutcome {
                chapter_id: ch.chapter_id.clone(),
                unit_id: u.unit_id.clone(),
                title: u.title.clone(),
                objective: u.objective.clone(),
                required: true,
                passed: false,
                state: "not_started".into(),
                current_attempt_id: "".into(),
            }],
        };
        validate_summary(&s, &c).unwrap();
        s.all_required_passed = true;
        assert!(validate_summary(&s, &c).is_err());
        s.all_required_passed = false;
        s.release_id = "other".into();
        assert!(validate_summary(&s, &c).is_err());
    }
}

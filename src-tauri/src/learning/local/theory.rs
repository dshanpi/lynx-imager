//! Activity-driven, ungraded reflection. Never manufactures execution evidence.
use super::*;

pub(super) fn default_activity_kind() -> String {
    "practice".into()
}

pub(super) fn is_reflection(lab: &LocalLab) -> bool {
    lab.activity_kind == "reflection"
}

pub(super) fn entry_requires_project(manifest: &LocalCourseManifest) -> bool {
    manifest.schema_version < 4
        || prerequisites::entry(manifest).is_some_and(|lab| !is_reflection(lab) && lab.required)
}

pub(super) fn validate_activity(
    manifest: &LocalCourseManifest,
    lab: &LocalLab,
) -> Result<(), String> {
    match lab.activity_kind.as_str() {
        "practice" => {
            if lab.verification_rules.len() != 1 || lab.required_evidence.is_empty() {
                return Err("LOCAL_COURSE_LABS_INVALID".into());
            }
            validate_command(&lab.verification_rules[0].command)
        }
        "reflection" if manifest.schema_version >= 4 => {
            if !lab.verification_rules.is_empty()
                || !lab.required_evidence.is_empty()
                || lab.hil_verification.is_some()
                || lab.risk_level != "reading-safe"
                || lab.document.is_none()
                || lab.comprehension_check.trim().is_empty()
                || lab.reflection_prompt.trim().is_empty()
            {
                return Err("LOCAL_COURSE_REFLECTION_INVALID".into());
            }
            Ok(())
        }
        _ => Err("LOCAL_COURSE_ACTIVITY_UNSUPPORTED".into()),
    }
}

pub(super) fn transition(lab: &LocalLab, from: &str, to: &str) -> bool {
    if is_reflection(lab) {
        matches!(
            (from, to),
            ("reading", "checking_understanding")
                | ("checking_understanding", "reflecting")
                | ("reflecting", "completed")
        )
    } else {
        transition_allowed(from, to)
    }
}

pub(super) fn require_answer(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(), String> {
    let answer: String = db.query_row("SELECT answer FROM learning_attempts WHERE session_id=?1 AND step_id=?2 ORDER BY attempt DESC LIMIT 1",
        params![session.session_id,session.current_step], |r| r.get(0)).map_err(|e| e.to_string())?;
    if answer.trim().is_empty() {
        return Err("LEARNING_ANSWER_REQUIRED".into());
    }
    Ok(())
}

pub(super) fn project_identity(
    work: &Connection,
    manifest: &LocalCourseManifest,
    project_id: i64,
) -> Result<(Option<i64>, String), String> {
    if project_id <= 0 {
        return Err("LEARNING_PROJECT_REQUIRED".into());
    }
    let project = crate::workbench::storage::project_by_id(work, project_id)
        .map_err(|_| "LEARNING_PROJECT_NOT_FOUND")?;
    if project.sdk_dir.trim().is_empty() {
        return Err("LEARNING_PROJECT_SDK_REQUIRED".into());
    }
    let required_scope = prerequisites::required_scope(manifest);
    match project.device_id {
        Some(id) => {
            let device = crate::workbench::storage::device_by_id(work, id)
                .map_err(|_| "LEARNING_DEVICE_NOT_FOUND")?;
            if device.board_uid.trim().is_empty() {
                return Err("LEARNING_BOARD_UID_REQUIRED".into());
            }
            Ok((Some(id), device.board_uid))
        }
        None if manifest
            .labs
            .iter()
            .any(|lab| required_scope.contains(&lab.step_id) && lab.hil_verification.is_some()) =>
        {
            Err("LEARNING_DEVICE_REQUIRED".into())
        }
        None => Ok((None, String::new())),
    }
}

pub(super) fn bind_project(
    db: &Connection,
    work: &Connection,
    session_id: &str,
    project_id: i64,
) -> Result<LocalLearningSession, String> {
    let session = get_session(db, session_id)?;
    let (manifest, lab) = manifest_and_lab(db, &session)?;
    if !matches!(manifest.schema_version, 4..=8)
        || session.project_id != 0
        || is_reflection(&lab)
        || !matches!(session.state.as_str(), "reading" | "checking_understanding")
    {
        return Err("LEARNING_PROJECT_BINDING_NOT_ALLOWED".into());
    }
    let (device_id, board_uid) = project_identity(work, &manifest, project_id)?;
    let changed = db.execute("UPDATE learning_sessions SET project_id=?1,device_id=?2,board_uid=?3,updated_at=?4 WHERE session_id=?5 AND project_id=0",
        params![project_id,device_id,board_uid,now(),session_id]).map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("LEARNING_PROJECT_BINDING_CHANGED".into());
    }
    get_session(db, session_id)
}

#[command]
pub fn learning_bind_local_project(
    session_id: String,
    project_id: i64,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<LocalLearningSession, String> {
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let work = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    bind_project(&db, &work, &session_id, project_id)
}

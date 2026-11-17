//! Explicit optional decisions are learning records, never execution evidence.
use super::*;

pub(super) fn required_by_default() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipRequest {
    session_id: String,
    expected_step_id: String,
    expected_attempt: i64,
    expected_state: String,
}

pub(super) fn advance(db: &Connection, session: &LocalLearningSession) -> Result<(), String> {
    let (manifest, _) = manifest_and_lab(db, session)?;
    let index = manifest
        .labs
        .iter()
        .position(|lab| lab.step_id == session.current_step)
        .ok_or("LEARNING_STEP_NOT_FOUND")?;
    let timestamp = now();
    let access = prerequisites::evaluate(db, session, &manifest)?;
    for next in manifest
        .labs
        .iter()
        .skip(index + 1)
        .chain(manifest.labs.iter().take(index))
    {
        if access
            .iter()
            .any(|a| a.step_id == next.step_id && !a.available)
        {
            continue;
        }
        let latest = revisit::latest(db, &session.session_id, &next.step_id)?;
        if latest
            .as_ref()
            .is_some_and(|(_, state)| matches!(state.as_str(), "completed" | "skipped"))
        {
            continue;
        }
        let next_state = latest
            .as_ref()
            .map(|(_, state)| state.as_str())
            .unwrap_or("reading");
        if !VALID_STATES.contains(&next_state) {
            return Err("LEARNING_STATE_INVALID".into());
        }
        db.execute("UPDATE learning_sessions SET current_step=?1,state=?2,classification='INCOMPLETE',updated_at=?3 WHERE session_id=?4", params![next.step_id,next_state,timestamp,session.session_id]).map_err(|e|e.to_string())?;
        if latest.is_none() {
            db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES(?1,?2,1,'reading',?3,?3)", params![session.session_id,next.step_id,timestamp]).map_err(|e|e.to_string())?;
        }
        return Ok(());
    }
    {
        let report = generate_local_report(db, &session.session_id)?;
        let classification = if report.required_completed_steps < report.required_steps {
            "INCOMPLETE"
        } else if report.required_steps == 0 {
            "OPTIONAL_COMPLETED"
        } else if report.software_result == "NOT_REQUIRED" {
            "REFLECTION_COMPLETED"
        } else if report.software_result == "PASS" {
            completion_classification(&report.hil_result)
        } else {
            "INCOMPLETE"
        };
        db.execute("UPDATE learning_sessions SET state='completed',classification=?1,updated_at=?2 WHERE session_id=?3", params![classification,timestamp,session.session_id]).map_err(|e|e.to_string())?;
    }
    Ok(())
}

fn skip(
    db: &mut Connection,
    actions: &execution::ActionGate,
    request: SkipRequest,
) -> Result<LocalLearningSession, String> {
    let session = get_session(db, &request.session_id)?;
    if session.state == "completed"
        || session.current_step != request.expected_step_id
        || session.state != request.expected_state
        || current_attempt(db, &session.session_id, &session.current_step)?
            != request.expected_attempt
    {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    let (manifest, lab) = manifest_and_lab(db, &session)?;
    if !matches!(manifest.schema_version, 6..=8) || lab.required {
        return Err("LEARNING_REQUIRED_ACTIVITY_CANNOT_SKIP".into());
    }
    let _lease = actions.acquire(session.project_id)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    append_evidence(
        &tx,
        &session,
        "optional-skip",
        "user-asserted",
        "RECORDED",
        "学员选择跳过选修活动；不计为完成或验证通过。",
        &serde_json::json!({"decision":"skipped","assessment":"NOT_GRADED"}),
    )?;
    tx.execute("UPDATE learning_attempts SET state='skipped',answer=COALESCE((SELECT answer FROM learning_drafts WHERE session_id=?2 AND step_id=?3 AND attempt=?4),answer),reflection=COALESCE((SELECT reflection FROM learning_drafts WHERE session_id=?2 AND step_id=?3 AND attempt=?4),reflection),updated_at=?1 WHERE session_id=?2 AND step_id=?3 AND attempt=?4", params![now(),session.session_id,session.current_step,request.expected_attempt]).map_err(|e|e.to_string())?;
    advance(&tx, &session)?;
    tx.commit().map_err(|e| e.to_string())?;
    get_session(db, &session.session_id)
}

#[command]
pub fn learning_skip_optional_activity(
    request: SkipRequest,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LocalLearningSession, String> {
    let mut db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    skip(&mut db, &state.actions, request)
}

#[cfg(test)]
pub(super) fn skip_for_test(
    db: &mut Connection,
    actions: &execution::ActionGate,
    session: &LocalLearningSession,
    attempt: i64,
) -> Result<LocalLearningSession, String> {
    skip(
        db,
        actions,
        SkipRequest {
            session_id: session.session_id.clone(),
            expected_step_id: session.current_step.clone(),
            expected_state: session.state.clone(),
            expected_attempt: attempt,
        },
    )
}

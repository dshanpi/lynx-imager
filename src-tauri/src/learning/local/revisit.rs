//! Explicit continuation decisions; viewing a chapter never calls this mutation.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SourcePlan {
    pub step_id: String,
    pub attempt: i64,
    pub digest: String,
    pub inputs_sha256: Option<String>,
    pub binding_sha256: Option<String>,
}
pub(super) fn source_plan(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<Vec<SourcePlan>, String> {
    let enabled:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM learning_evidence WHERE session_id=?1 AND kind='activity-navigation')",[&session.session_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let (manifest, _) = manifest_and_lab(db, session)?;
    if !enabled && manifest.schema_version < 8 {
        return Ok(Vec::new());
    }
    let scope = prerequisites::required_scope(&manifest);
    let mut plans = Vec::new();
    for lab in manifest
        .labs
        .iter()
        .filter(|l| scope.contains(&l.step_id) && !theory::is_reflection(l))
    {
        let attempt = current_attempt(db, &session.session_id, &lab.step_id)?;
        let (digest,status):(String,String)=db.query_row("SELECT digest,status FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1",params![session.session_id,lab.step_id,attempt],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|environment_receipt::UNAVAILABLE)?;
        if status != "PASS" {
            return Err(environment_receipt::UNAVAILABLE.into());
        }
        let mut at = session.clone();
        at.current_step = lab.step_id.clone();
        let (inputs_sha256, binding_sha256) = match freshness::saved(db, &at) {
            Ok((saved, snapshot)) if saved == digest => {
                (snapshot.inputs_sha256, Some(snapshot.binding_sha256))
            }
            Err(e) if e.starts_with("LEARNING_INPUTS_UNRECORDED") => (None, None),
            _ => return Err(environment_receipt::UNAVAILABLE.into()),
        };
        plans.push(SourcePlan {
            step_id: lab.step_id.clone(),
            attempt,
            digest,
            inputs_sha256,
            binding_sha256,
        });
    }
    Ok(plans)
}
pub(super) fn check_sources(
    plans: &[SourcePlan],
    current: &str,
    bound: &str,
) -> Vec<environment_report::Currency> {
    plans
        .iter()
        .map(|p| environment_report::Currency {
            step_id: p.step_id.clone(),
            attempt: p.attempt,
            digest: p.digest.clone(),
            status: if p.inputs_sha256.is_none() {
                "UNRECORDED"
            } else if p.inputs_sha256.as_deref() != Some(current)
                || p.binding_sha256.as_deref() != Some(bound)
            {
                "CHANGED"
            } else {
                "MATCH"
            }
            .into(),
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    session_id: String,
    expected_step_id: String,
    expected_attempt: i64,
    expected_state: String,
    target_step_id: String,
    expected_target_attempt: i64,
}

pub(super) fn latest(
    db: &Connection,
    session: &str,
    step: &str,
) -> Result<Option<(i64, String)>, String> {
    db.query_row("SELECT attempt,state FROM learning_attempts WHERE session_id=?1 AND step_id=?2 ORDER BY attempt DESC LIMIT 1", params![session,step], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())
}

pub(super) fn select(
    db: &mut Connection,
    actions: &execution::ActionGate,
    request: Request,
) -> Result<LocalLearningSession, String> {
    let session = get_session(db, &request.session_id)?;
    if session.current_step != request.expected_step_id
        || session.state != request.expected_state
        || current_attempt(db, &session.session_id, &session.current_step)?
            != request.expected_attempt
    {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    let (manifest, _) = manifest_and_lab(db, &session)?;
    if !manifest
        .labs
        .iter()
        .any(|l| l.step_id == request.target_step_id)
    {
        return Err("LEARNING_STEP_NOT_FOUND".into());
    }
    prerequisites::require(db, &session, &manifest, &request.target_step_id)?;
    let found = latest(db, &session.session_id, &request.target_step_id)?;
    if found.is_none() && manifest.schema_version < 8 {
        return Err("LEARNING_ACTIVITY_NOT_VISITED:请按当前课程顺序开始未访问的课时".into());
    }
    let first_visit = found.is_none();
    let (attempt, state) = found.unwrap_or((0, "reading".into()));
    if attempt != request.expected_target_attempt
        || (!first_visit && attempt <= 0)
        || (!VALID_STATES.contains(&state.as_str()) && state != "skipped")
    {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    let restart = first_visit || matches!(state.as_str(), "completed" | "skipped");
    if !restart && session.current_step == request.target_step_id {
        return Err("LEARNING_ACTIVITY_ALREADY_CURRENT".into());
    }
    let _lease = actions.acquire(session.project_id)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let next_attempt = if restart {
        attempt.checked_add(1).ok_or("LEARNING_ATTEMPT_LIMIT")?
    } else {
        attempt
    };
    let next_state = if restart { "reading" } else { state.as_str() };
    let timestamp = now();
    if restart {
        tx.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES(?1,?2,?3,'reading',?4,?4)",params![session.session_id,request.target_step_id,next_attempt,timestamp]).map_err(|e|e.to_string())?;
    }
    append_evidence(
        &tx,
        &session,
        "activity-navigation",
        "user-asserted",
        "RECORDED",
        if first_visit {
            "学员选择开始前置条件已满足的活动；创建首次尝试，尚未验证。"
        } else if restart {
            "学员选择重新学习已结束课时；创建新尝试，历史记录保留，尚未验证。"
        } else {
            "学员选择继续已访问课时；恢复原尝试，未执行课程动作。"
        },
        &serde_json::json!({"targetStepId":request.target_step_id,"targetAttempt":next_attempt,"previousTargetAttempt":attempt,"newAttempt":restart,"firstVisit":first_visit,"executedAction":false}),
    )?;
    tx.execute("UPDATE learning_sessions SET current_step=?1,state=?2,classification='INCOMPLETE',updated_at=?3 WHERE session_id=?4",params![request.target_step_id,next_state,timestamp,session.session_id]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    get_session(db, &session.session_id)
}

#[command]
pub fn learning_revisit_activity(
    request: Request,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LocalLearningSession, String> {
    let mut db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    select(&mut db, &state.actions, request)
}

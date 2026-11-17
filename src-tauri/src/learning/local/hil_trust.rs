//! Local HIL currently has only user-entered observations, not device adapter receipts.
//! Never turn claimed PASS/duration/image hashes into verified physical execution.
use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedEvidence {
    source: String,
    status: String,
}

pub(super) fn project_evidence(
    source: &mut String,
    status: &mut String,
    summary: &mut String,
) -> Option<RecordedEvidence> {
    let original = RecordedEvidence {
        source: source.clone(),
        status: status.clone(),
    };
    match source.as_str() {
        "hil-device" => {
            *source = "user-asserted".into();
            *summary = format!("历史人工观察（原记录 {}，非设备验证）：{}", status, summary);
            *status = "RECORDED".into();
        }
        "hil-validator" => {
            *source = "legacy-hil-assessment".into();
            *summary = format!(
                "旧规则结论（原记录 {}，缺可信设备回执）：{}",
                status, summary
            );
            *status = "NOT_VERIFIED".into();
        }
        _ => return None,
    }
    Some(original)
}

pub(super) fn project_session(mut session: LocalLearningSession) -> LocalLearningSession {
    if session.classification == "FULL_PASS" {
        session.classification = "HIL_NOT_VERIFIED".into();
    }
    session
}

pub(super) fn context(
    db: &Connection,
    session: &LocalLearningSession,
    request: &HilEvidenceRequest,
) -> Result<(), String> {
    if request.expected_step_id.as_deref() != Some(session.current_step.as_str())
        || request.expected_attempt
            != Some(current_attempt(
                db,
                &session.session_id,
                &session.current_step,
            )?)
    {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    Ok(())
}

pub(super) fn assessment(
    db: &mut Connection,
    actions: &execution::ActionGate,
    session_id: &str,
) -> Result<StepActionResult, String> {
    let session = get_session(db, session_id)?;
    if session.state != "practicing" {
        return Err("LEARNING_STEP_NOT_PRACTICING".into());
    }
    let _lease = actions.acquire(session.project_id)?;
    let (_, lab) = manifest_and_lab(db, &session)?;
    let hil = lab.hil_verification.ok_or("LEARNING_HIL_NOT_REQUIRED")?;
    if latest_verification(db, session_id, &session.current_step, "command", "software")?.as_deref()
        != Some("PASS")
    {
        return Err("LEARNING_SOFTWARE_VERIFICATION_REQUIRED".into());
    }
    // Every existing local HIL producer is the manual observation command. A
    // typed device adapter must supply independently verified, scoped receipts
    // before this assessment can award a physical PASS. Do not invent one here.
    let summary="尚未验证实机：当前本地课程只保存人工观察，缺少可信设备执行与采集回执。手填结果、镜像摘要和持续时间不能替代实机验证。";
    let digest = append_evidence(
        db,
        &session,
        "hil-verification",
        "hil-assessment",
        "NOT_VERIFIED",
        summary,
        &serde_json::json!({"trustVersion":2,"reason":"DEVICE_ADAPTER_REQUIRED","actionId":hil.action_id,"boardUid":session.board_uid,"requiredEvidenceKinds":hil.required_evidence_kinds}),
    )?;
    Ok(StepActionResult {
        status: "NOT_VERIFIED".into(),
        source: "hil-assessment".into(),
        exit_code: None,
        output_digest: digest,
        summary: summary.into(),
    })
}

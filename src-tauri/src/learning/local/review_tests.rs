use super::*;

fn review_database() -> (Connection, LocalCourseManifest, LocalLearningSession) {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let manifest = super::tests::manifest_v2_with_hil();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','T527','board','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('review','t527-course','1.0.0-rc.1',1,?1,'collecting_evidence',1,1)", [&manifest.labs[1].step_id]).unwrap();
    let session = get_session(&db, "review").unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('review',?1,1,'completed',1,1)", [&session.current_step]).unwrap();
    append_evidence(
        &db,
        &session,
        "hil-verification",
        "hil-validator",
        "PASS",
        "old pass",
        &serde_json::json!({}),
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "old pass",
        &serde_json::json!({}),
    )
    .unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('review',?1,2,'collecting_evidence',2,2)", [&session.current_step]).unwrap();
    (db, manifest, session)
}

#[test]
fn review_hil_report_rejects_previous_attempt_pass() {
    let (db, manifest, session) = review_database();
    assert_eq!(
        hil_report_status(&db, &session, &manifest).unwrap(),
        ("NOT_VERIFIED".into(), 0, 1)
    );
}

#[test]
fn review_current_step_excludes_previous_attempt_evidence() {
    let (db, _, session) = review_database();
    assert!(list_step_evidence(&db, "review", &session.current_step)
        .unwrap()
        .is_empty());
}

#[test]
fn review_transition_requires_current_software_and_rejects_legacy_hil_pass() {
    let (mut db, _, session) = review_database();
    append_evidence(
        &db,
        &session,
        "photo",
        "camera",
        "RECORDED",
        "photo",
        &serde_json::json!({}),
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "tutor",
        "ai-provider",
        "PASS",
        "not a validator",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(
        transition_step(&mut db, "review", "verifying", None, None).unwrap_err(),
        "LEARNING_SOFTWARE_VERIFICATION_REQUIRED"
    );
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "current software",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(
        transition_step(&mut db, "review", "verifying", None, None).unwrap_err(),
        "LEARNING_HIL_VERIFICATION_REQUIRED:需要可信设备适配器证据；人工观察与旧规则通过不能替代"
    );
    append_evidence(
        &db,
        &session,
        "hil-verification",
        "hil-validator",
        "PASS",
        "current HIL",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(
        transition_step(&mut db, "review", "verifying", None, None).unwrap_err(),
        "LEARNING_HIL_VERIFICATION_REQUIRED:需要可信设备适配器证据；人工观察与旧规则通过不能替代"
    );
    assert_eq!(
        get_session(&db, "review").unwrap().state,
        "collecting_evidence"
    );
    // A session already advanced by old rules must still support an explicit retry.
    db.execute(
        "UPDATE learning_sessions SET state='verifying' WHERE session_id='review'",
        [],
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "FAIL",
        "latest result wins",
        &serde_json::json!({}),
    )
    .unwrap();
    assert!(transition_step(&mut db, "review", "reflecting", None, None).is_err());
    assert_eq!(get_session(&db, "review").unwrap().state, "verifying");
    assert_eq!(
        transition_step(&mut db, "review", "practicing", None, None)
            .unwrap()
            .state,
        "practicing"
    );
    assert_eq!(
        current_attempt(&db, "review", &session.current_step).unwrap(),
        3
    );
}

#[test]
fn review_submitted_answers_are_redacted() {
    let (mut db, _, session) = review_database();
    db.execute(
        "UPDATE learning_sessions SET state='checking_understanding' WHERE session_id='review'",
        [],
    )
    .unwrap();
    transition_step(
        &mut db,
        "review",
        "practicing",
        Some("password=secret-demo-value".into()),
        Some("token=secret-demo-token".into()),
    )
    .unwrap();
    let (answer, reflection): (String, String) = db.query_row("SELECT answer,reflection FROM learning_attempts WHERE session_id='review' AND step_id=?1 AND attempt=2", [&session.current_step], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(!answer.contains("secret-demo-value"));
    assert!(!reflection.contains("secret-demo-token"));
    assert!(answer.contains("[REDACTED]"));
}

#[test]
fn review_session_insert_rolls_back_if_attempt_insert_fails() {
    let (db, _, mut session) = review_database();
    session.session_id = "new-session".into();
    db.execute_batch("CREATE TRIGGER reject_attempt BEFORE INSERT ON learning_attempts BEGIN SELECT RAISE(ABORT,'test attempt failure'); END;").unwrap();
    assert!(insert_session(&db, &session).is_err());
    assert!(get_session(&db, "new-session").is_err());
}

#[test]
fn review_action_result_rejects_changed_state_step_and_attempt() {
    for change in [
        "UPDATE learning_sessions SET state='verifying' WHERE session_id='review'",
        "UPDATE learning_sessions SET current_step='step-3' WHERE session_id='review'",
        "INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('review','step-2',3,'practicing',3,3)",
    ] {
        let (mut db, _, original) = review_database();
        db.execute(change, []).unwrap();
        assert_eq!(record_action_result(&mut db, &original, 2, 0, 0, b"ok", false).unwrap_err(), "LEARNING_ACTION_CONTEXT_CHANGED");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get::<_,i64>(0)).unwrap(), 2);
    }
}

#[test]
fn review_action_commit_rolls_back_evidence_and_both_states_on_failure() {
    let (mut db, _, session) = review_database();
    db.execute_batch("CREATE TRIGGER reject_state BEFORE UPDATE ON learning_attempts BEGIN SELECT RAISE(ABORT,'test state failure'); END;").unwrap();
    assert!(record_action_result(&mut db, &session, 2, 0, 0, b"ok", false).is_err());
    assert!(list_step_evidence(&db, "review", &session.current_step)
        .unwrap()
        .is_empty());
    assert_eq!(get_session(&db, "review").unwrap().updated_at, 1);
}

#[test]
fn review_action_pass_failure_and_error_have_distinct_results() {
    let (mut db, _, mut session) = review_database();
    db.execute(
        "UPDATE learning_sessions SET state='practicing' WHERE session_id='review'",
        [],
    )
    .unwrap();
    session.state = "practicing".into();
    assert_eq!(
        record_action_result(&mut db, &session, 2, 1, 0, b"failed", false)
            .unwrap()
            .status,
        "FAIL"
    );
    assert_eq!(get_session(&db, "review").unwrap().state, "practicing");
    assert_eq!(
        record_action_result(&mut db, &session, 2, 0, 0, b"password=test-secret", true)
            .unwrap()
            .status,
        "PASS"
    );
    assert_eq!(get_session(&db, "review").unwrap().state, "practicing");
    record_action_failure(&db, &session, 2, "timeout token=test-secret").unwrap();
    assert_eq!(
        latest_verification(&db, "review", &session.current_step, "command", "software")
            .unwrap()
            .as_deref(),
        Some("ERROR")
    );
    assert!(require_current_verification(&db, &session).is_err());
    let details: Vec<String> = db
        .prepare("SELECT detail_json FROM learning_evidence WHERE attempt=2")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(details.iter().all(|text| !text.contains("test-secret")));
    record_action_result(&mut db, &session, 2, 0, 0, b"ok", false).unwrap();
    assert_eq!(
        get_session(&db, "review").unwrap().state,
        "collecting_evidence"
    );
    let state: String = db
        .query_row(
            "SELECT state FROM learning_attempts WHERE attempt=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "collecting_evidence");
}

#[test]
fn review_host_only_course_completes_and_report_ignores_old_attempt() {
    let (mut db, mut manifest, session) = review_database();
    assert_eq!(
        generate_local_report(&db, "review")
            .unwrap()
            .completed_steps,
        0
    );
    manifest.labs = vec![manifest.labs[1].clone()];
    manifest.labs[0].hil_verification = None;
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&manifest).unwrap()],
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "pass",
        &serde_json::json!({}),
    )
    .unwrap();
    transition_step(&mut db, "review", "verifying", None, None).unwrap();
    transition_step(&mut db, "review", "reflecting", None, None).unwrap();
    let finished = transition_step(
        &mut db,
        "review",
        "completed",
        None,
        Some("学会了定位错误".into()),
    )
    .unwrap();
    assert_eq!(finished.classification, "SOFTWARE_PASS");
    let report = generate_local_report(&db, "review").unwrap();
    assert_eq!(report.completed_steps, 1);
    assert_eq!(report.software_result, "PASS");
    assert_eq!(report.hil_result, "NOT_REQUIRED");
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "FAIL",
        "invalidated",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(
        generate_local_report(&db, "review")
            .unwrap()
            .software_result,
        "INCOMPLETE"
    );
}

#[test]
fn review_manifest_keeps_exactly_one_verification_rule() {
    let mut manifest = super::tests::manifest_v2_with_hil();
    let duplicate = manifest.labs[1].verification_rules[0].clone();
    manifest.labs[0].verification_rules.push(duplicate);
    assert!(validate_manifest(&manifest).is_err());
}

#[test]
fn review_redaction_preserves_explanation_and_covers_token_variants() {
    let clean = crate::supervisor::redact_learning_context(
        "token=secret-a access_token=secret-b refresh_token=secret-c；普通分析 token count 5",
    );
    assert!(!clean.contains("secret-"));
    assert!(clean.contains("token count 5"));
}

#[test]
fn review_freshness_recovery_creates_new_attempt_and_preserves_history() {
    for state in ["collecting_evidence", "verifying", "reflecting"] {
        let (mut db, _, session) = review_database();
        db.execute(
            "UPDATE learning_sessions SET state=?1 WHERE session_id='review'",
            [state],
        )
        .unwrap();
        db.execute(
            "UPDATE learning_attempts SET state=?1 WHERE session_id='review' AND attempt=2",
            [state],
        )
        .unwrap();
        let before: i64 = db
            .query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get(0))
            .unwrap();
        let result = transition_step(&mut db, "review", "practicing", None, None).unwrap();
        assert_eq!(result.state, "practicing");
        assert_eq!(
            current_attempt(&db, "review", &session.current_step).unwrap(),
            3
        );
        assert!(list_step_evidence(&db, "review", &session.current_step)
            .unwrap()
            .is_empty());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            before
        );
    }
}

use super::*;

#[test]
fn revisit_source_checks_do_not_let_new_early_execution_hide_old_later_sources() {
    let plans = vec![
        revisit::SourcePlan {
            step_id: "early".into(),
            attempt: 2,
            digest: "new".into(),
            inputs_sha256: Some("b".repeat(64)),
            binding_sha256: Some("c".repeat(64)),
        },
        revisit::SourcePlan {
            step_id: "later".into(),
            attempt: 1,
            digest: "old".into(),
            inputs_sha256: Some("a".repeat(64)),
            binding_sha256: Some("c".repeat(64)),
        },
    ];
    let checks = revisit::check_sources(&plans, &"b".repeat(64), &"c".repeat(64));
    assert_eq!(checks[0].status, "MATCH");
    assert_eq!(checks[1].status, "CHANGED");
    assert_eq!(environment_report::aggregate("MATCH", &checks), "CHANGED");
    let mut legacy = plans[1].clone();
    legacy.inputs_sha256 = None;
    assert_eq!(
        revisit::check_sources(&[legacy], &"b".repeat(64), &"c".repeat(64))[0].status,
        "UNRECORDED"
    );
}

fn database() -> (Connection, LocalLearningSession) {
    let (db, session) = report_tests::database();
    let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
    manifest.schema_version = 7;
    let mut second = manifest.labs[0].clone();
    second.step_id = "second".into();
    second.number = 2;
    manifest.labs.push(second);
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&manifest).unwrap()],
    )
    .unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,answer,reflection,created_at,updated_at) VALUES('r','second',1,'checking_understanding','saved answer','saved reflection',1,1)", []).unwrap();
    (db, session)
}

#[test]
fn revisit_advancement_resumes_existing_attempt_instead_of_inserting_duplicate() {
    let (db, session) = database();
    optional::advance(&db, &session).unwrap();
    let resumed = get_session(&db, "r").unwrap();
    assert_eq!(resumed.current_step, "second");
    assert_eq!(resumed.state, "checking_understanding");
    assert_eq!(current_attempt(&db, "r", "second").unwrap(), 1);
    let answer: String = db
        .query_row(
            "SELECT answer FROM learning_attempts WHERE step_id='second'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(answer, "saved answer");
}

fn request(
    session: &LocalLearningSession,
    origin_attempt: i64,
    target: &str,
    target_attempt: i64,
) -> revisit::Request {
    serde_json::from_value(serde_json::json!({"sessionId":session.session_id,"expectedStepId":session.current_step,"expectedAttempt":origin_attempt,"expectedState":session.state,"targetStepId":target,"expectedTargetAttempt":target_attempt})).unwrap()
}

#[test]
fn revisit_completed_activity_creates_clean_attempt_preserving_history_and_report_truth() {
    let (mut db, session) = database();
    db.execute(
        "INSERT INTO learning_drafts VALUES('r','step-1',1,'old draft','old reflection',1)",
        [],
    )
    .unwrap();
    let old: String = db
        .query_row(
            "SELECT detail_json FROM learning_evidence LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let selected = revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&session, 1, "step-1", 1),
    )
    .unwrap();
    assert_eq!(selected.current_step, "step-1");
    assert_eq!(selected.state, "reading");
    assert_eq!(selected.classification, "INCOMPLETE");
    assert_eq!(current_attempt(&db, "r", "step-1").unwrap(), 2);
    assert_eq!(
        db.query_row(
            "SELECT answer FROM learning_attempts WHERE step_id='step-1' AND attempt=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        ""
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM learning_attempts WHERE step_id='step-1' AND attempt=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "completed"
    );
    assert_eq!(
        db.query_row(
            "SELECT answer FROM learning_drafts WHERE step_id='step-1' AND attempt=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "old draft"
    );
    assert_eq!(
        db.query_row(
            "SELECT detail_json FROM learning_evidence WHERE kind='command' LIMIT 1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        old
    );
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.software_result, "INCOMPLETE");
    assert_eq!(report.completed_steps, 0);
    assert!(revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&session, 1, "step-1", 1)
    )
    .is_err());
}

#[test]
fn revisit_resumes_unfinished_target_without_copying_another_steps_draft() {
    let (mut db, session) = database();
    db.execute(
        "INSERT INTO learning_drafts VALUES('r','second',1,'target draft','target reflection',1)",
        [],
    )
    .unwrap();
    let selected = revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&session, 1, "second", 1),
    )
    .unwrap();
    assert_eq!(selected.state, "checking_understanding");
    assert_eq!(selected.current_step, "second");
    assert_eq!(current_attempt(&db, "r", "second").unwrap(), 1);
    let context = journey::agent_context(&db, &selected).unwrap();
    assert_eq!(context["answer"], "target draft");
    assert!(revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&selected, 1, "second", 1)
    )
    .unwrap_err()
    .starts_with("LEARNING_ACTIVITY_ALREADY_CURRENT"));
}

#[test]
fn revisit_rejects_stale_unknown_unvisited_and_busy_requests_without_writes() {
    let (mut db, session) = database();
    let gate = execution::ActionGate::default();
    for req in [
        request(&session, 2, "second", 1),
        request(&session, 1, "second", 2),
        request(&session, 1, "foreign", 1),
    ] {
        let before = db.total_changes();
        assert!(revisit::select(&mut db, &gate, req).is_err());
        assert_eq!(db.total_changes(), before);
    }
    db.execute("DELETE FROM learning_attempts WHERE step_id='second'", [])
        .unwrap();
    let before = db.total_changes();
    assert!(
        revisit::select(&mut db, &gate, request(&session, 1, "second", 0))
            .unwrap_err()
            .starts_with("LEARNING_ACTIVITY_NOT_VISITED")
    );
    assert_eq!(db.total_changes(), before);
    let _lease = gate.acquire(session.project_id).unwrap();
    assert!(
        revisit::select(&mut db, &gate, request(&session, 1, "step-1", 1))
            .unwrap_err()
            .starts_with("LEARNING_ACTION_BUSY")
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn revisit_transaction_failure_preserves_attempts_and_origin() {
    let (mut db, session) = database();
    db.execute_batch("CREATE TRIGGER fail_navigation BEFORE UPDATE ON learning_sessions BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
    assert!(revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&session, 1, "step-1", 1)
    )
    .is_err());
    assert_eq!(current_attempt(&db, "r", "step-1").unwrap(), 1);
    assert_eq!(get_session(&db, "r").unwrap().state, "completed");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM learning_evidence WHERE kind='activity-navigation'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn revisit_skipped_optional_creates_new_attempt_and_completion_skips_ended_targets() {
    let (mut db, session) = database();
    db.execute(
        "UPDATE learning_attempts SET state='skipped' WHERE step_id='second'",
        [],
    )
    .unwrap();
    let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
    manifest.labs[1].required = false;
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&manifest).unwrap()],
    )
    .unwrap();
    let selected = revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&session, 1, "second", 1),
    )
    .unwrap();
    assert_eq!(selected.state, "reading");
    assert_eq!(current_attempt(&db, "r", "second").unwrap(), 2);
    assert_eq!(
        generate_local_report(&db, "r")
            .unwrap()
            .optional_skipped_steps,
        0
    );
    db.execute(
        "UPDATE learning_attempts SET state='completed' WHERE step_id='second' AND attempt=2",
        [],
    )
    .unwrap();
    optional::advance(&db, &selected).unwrap();
    assert_eq!(get_session(&db, "r").unwrap().state, "completed");
    assert_eq!(current_attempt(&db, "r", "step-1").unwrap(), 1);
}

#[test]
fn revisit_advancement_wraps_to_earlier_unfinished_and_never_reuses_other_session() {
    let (db, session) = database();
    db.execute(
        "UPDATE learning_attempts SET state='practicing' WHERE step_id='step-1'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE learning_attempts SET state='completed' WHERE step_id='second'",
        [],
    )
    .unwrap();
    let mut later = session;
    later.current_step = "second".into();
    optional::advance(&db, &later).unwrap();
    let current = get_session(&db, "r").unwrap();
    assert_eq!(current.current_step, "step-1");
    assert_eq!(current.state, "practicing");
    db.execute(
        "DELETE FROM learning_attempts WHERE session_id='r' AND step_id='second'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('foreign','second',7,'completed',1,1)",[]).unwrap();
    db.execute(
        "UPDATE learning_attempts SET state='completed' WHERE session_id='r' AND step_id='step-1'",
        [],
    )
    .unwrap();
    optional::advance(&db, &current).unwrap();
    assert_eq!(current_attempt(&db, "r", "second").unwrap(), 1);
    assert_eq!(get_session(&db, "r").unwrap().state, "reading");
}

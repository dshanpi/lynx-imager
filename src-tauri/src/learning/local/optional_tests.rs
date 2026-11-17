use super::*;

fn manifest(version: u32, required: Option<bool>) -> LocalCourseManifest {
    let mut value = serde_json::to_value(tests::manifest_v2_with_hil()).unwrap();
    value["schemaVersion"] = version.into();
    value["workspace"] =
        serde_json::json!({"remoteSubdir":"lesson","packageSourceRoot":"workspace"});
    value["labs"] = serde_json::json!([value["labs"][0].clone()]);
    if let Some(required) = required {
        value["labs"][0]["required"] = required.into();
    }
    serde_json::from_value(value).unwrap()
}

#[test]
fn optional_schema6_accepts_signed_requirement_flag() {
    validate_manifest(&manifest(6, Some(false))).unwrap();
}

#[test]
fn optional_legacy_schema_must_not_silently_ignore_false() {
    assert!(validate_manifest(&manifest(5, Some(false))).is_err());
    validate_manifest(&manifest(5, None)).unwrap();
}

fn database(required: bool) -> (Connection, LocalLearningSession) {
    let (db, mut session) = report_tests::database();
    let value = manifest(6, Some(required));
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    db.execute("DELETE FROM learning_evidence", []).unwrap();
    db.execute(
        "UPDATE learning_sessions SET project_id=0,state='reading'",
        [],
    )
    .unwrap();
    db.execute("UPDATE learning_attempts SET state='reading',answer='preserved answer',reflection='preserved reflection'", []).unwrap();
    session.project_id = 0;
    session.state = "reading".into();
    (db, session)
}

#[test]
fn optional_defaults_required_and_rejects_null() {
    let mut value = serde_json::to_value(manifest(5, None)).unwrap();
    value["labs"][0].as_object_mut().unwrap().remove("required");
    let old: LocalCourseManifest = serde_json::from_value(value.clone()).unwrap();
    assert!(old.labs[0].required);
    value["labs"][0]["required"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<LocalCourseManifest>(value).is_err());
    for version in 1..=5 {
        let mut old = tests::manifest_v2_with_hil();
        old.schema_version = version;
        old.labs[0].required = false;
        assert!(validate_manifest(&old).is_err());
    }
}

#[test]
fn optional_skip_without_project_keeps_failure_and_never_creates_pass() {
    let (mut db, session) = database(false);
    let actions = execution::ActionGate::default();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "FAIL",
        "real failure preserved",
        &serde_json::json!({}),
    )
    .unwrap();
    let done = optional::skip_for_test(&mut db, &actions, &session, 1).unwrap();
    assert_eq!(done.state, "completed");
    assert_eq!(done.classification, "OPTIONAL_COMPLETED");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(
        (
            report.required_steps,
            report.completed_steps,
            report.optional_skipped_steps
        ),
        (0, 0, 1)
    );
    assert_eq!(report.software_result, "NOT_REQUIRED");
    assert_eq!(report.hil_result, "NOT_REQUIRED");
    let rows: (String, String, String) = db
        .query_row(
            "SELECT state,answer,reflection FROM learning_attempts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        rows,
        (
            "skipped".into(),
            "preserved answer".into(),
            "preserved reflection".into()
        )
    );
    let facts: (i64, i64, i64) = db
        .query_row(
            "SELECT COUNT(*),SUM(status='FAIL'),SUM(status='PASS') FROM learning_evidence",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(facts, (2, 1, 0));
    assert!(optional::skip_for_test(&mut db, &actions, &session, 1).is_err());
    assert!(optional::skip_for_test(&mut db, &actions, &done, 1).is_err());
    assert_eq!(generate_local_report(&db, "r").unwrap().evidence_count, 2);
}

#[test]
fn optional_skip_rejects_required_busy_and_stale_without_writes() {
    let actions = execution::ActionGate::default();
    let (mut db, session) = database(true);
    assert_eq!(
        optional::skip_for_test(&mut db, &actions, &session, 1).unwrap_err(),
        "LEARNING_REQUIRED_ACTIVITY_CANNOT_SKIP"
    );
    let (mut db, session) = database(false);
    assert!(optional::skip_for_test(&mut db, &actions, &session, 2).is_err());
    let mut stale = session.clone();
    stale.current_step = "other".into();
    assert!(optional::skip_for_test(&mut db, &actions, &stale, 1).is_err());
    stale = session.clone();
    stale.state = "practicing".into();
    assert!(optional::skip_for_test(&mut db, &actions, &stale, 1).is_err());
    let lease = actions.acquire(0).unwrap();
    assert_eq!(
        optional::skip_for_test(&mut db, &actions, &session, 1).unwrap_err(),
        "LEARNING_ACTION_BUSY:该工程已有课程命令运行，请等待结果"
    );
    assert_eq!(generate_local_report(&db, "r").unwrap().evidence_count, 0);
    assert_eq!(get_session(&db, "r").unwrap().state, "reading");
    drop(lease);
    optional::skip_for_test(&mut db, &actions, &session, 1).unwrap();
}

#[test]
fn optional_skip_advances_to_required_and_cannot_replace_its_verification() {
    let (mut db, session) = database(false);
    let (mut value, _) = manifest_and_lab(&db, &session).unwrap();
    let mut required = value.labs[0].clone();
    required.required = true;
    required.number = 2;
    required.step_id = "required".into();
    value.labs.push(required);
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    let next =
        optional::skip_for_test(&mut db, &execution::ActionGate::default(), &session, 1).unwrap();
    assert_eq!(next.current_step, "required");
    assert_eq!(next.state, "reading");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.required_steps, 1);
    assert_eq!(report.required_completed_steps, 0);
    assert_eq!(report.optional_skipped_steps, 1);
    assert_eq!(report.software_result, "INCOMPLETE");
    assert!(transition_step(&mut db, "r", "completed", None, Some("skip".into())).is_err());
}

#[test]
fn optional_advance_failure_rolls_back_skip_record_and_attempt() {
    let (mut db, session) = database(false);
    let (mut value, _) = manifest_and_lab(&db, &session).unwrap();
    let mut next = value.labs[0].clone();
    next.number = 2;
    next.step_id = "next".into();
    value.labs.push(next);
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','next',1,'reading',1,1)",[]).unwrap();
    // Existing attempts now resume normally. Inject a real write failure instead;
    // the same assertions must still prove rollback of the skip and its evidence.
    db.execute_batch("CREATE TRIGGER reject_advance BEFORE UPDATE ON learning_sessions WHEN NEW.current_step='next' BEGIN SELECT RAISE(ABORT,'fixture advance storage failure'); END;").unwrap();
    assert!(
        optional::skip_for_test(&mut db, &execution::ActionGate::default(), &session, 1).is_err()
    );
    let current = get_session(&db, "r").unwrap();
    assert_eq!(current.current_step, session.current_step);
    assert_eq!(current.state, "reading");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.evidence_count, 0);
    assert_eq!(report.optional_skipped_steps, 0);
}

#[test]
fn optional_skip_keeps_latest_draft_in_readable_history_without_grading() {
    let (mut db, session) = database(false);
    db.execute(
        "INSERT INTO learning_drafts VALUES('r','step-1',1,'draft answer','draft reflection',2)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO learning_drafts VALUES('r','step-1',2,'other attempt','other attempt',3)",
        [],
    )
    .unwrap();
    optional::skip_for_test(&mut db, &execution::ActionGate::default(), &session, 1).unwrap();
    let saved: (String, String) = db
        .query_row(
            "SELECT answer,reflection FROM learning_attempts WHERE attempt=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, ("draft answer".into(), "draft reflection".into()));
    assert_eq!(
        generate_local_report(&db, "r")
            .unwrap()
            .optional_completed_steps,
        0
    );
    let evidence: (String, String) = db
        .query_row("SELECT status,source FROM learning_evidence", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(evidence, ("RECORDED".into(), "user-asserted".into()));
}

#[test]
fn optional_reflection_can_be_completed_instead_of_skipped() {
    let (mut db, _) = database(false);
    let mut value = manifest(6, Some(false));
    let lab = &mut value.labs[0];
    lab.activity_kind = "reflection".into();
    lab.risk_level = "reading-safe".into();
    lab.verification_rules.clear();
    lab.required_evidence.clear();
    lab.document = Some(LearnDocumentSpec {
        path: "docs/theory.md".into(),
        sha256: "a".repeat(64),
        source_url: "https://example.invalid/theory".into(),
    });
    value.workspace = None;
    validate_manifest(&value).unwrap();
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    transition_step(&mut db, "r", "checking_understanding", None, None).unwrap();
    transition_step(
        &mut db,
        "r",
        "reflecting",
        Some("my understanding".into()),
        None,
    )
    .unwrap();
    let done = transition_step(
        &mut db,
        "r",
        "completed",
        None,
        Some("my reflection".into()),
    )
    .unwrap();
    assert_eq!(done.classification, "OPTIONAL_COMPLETED");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(
        (
            report.completed_steps,
            report.optional_completed_steps,
            report.optional_skipped_steps
        ),
        (1, 1, 0)
    );
    assert_eq!(report.software_result, "NOT_REQUIRED");
    assert_eq!(report.hil_result, "NOT_REQUIRED");
}

#[test]
fn optional_failed_practice_and_hil_do_not_block_required_report() {
    let (db, session) = report_tests::database();
    let (mut value, _) = manifest_and_lab(&db, &session).unwrap();
    value.schema_version = 6;
    let mut optional = tests::manifest_v2_with_hil().labs[1].clone();
    optional.required = false;
    value.labs.push(optional.clone());
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    let mut other = session.clone();
    other.current_step = optional.step_id.clone();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r',?1,1,'skipped',1,1)",[&optional.step_id]).unwrap();
    append_evidence(
        &db,
        &other,
        "command",
        "software",
        "FAIL",
        "failure",
        &serde_json::json!({}),
    )
    .unwrap();
    append_evidence(
        &db,
        &other,
        "hil-verification",
        "hil-validator",
        "FAIL",
        "failure",
        &serde_json::json!({}),
    )
    .unwrap();
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(
        (
            report.required_steps,
            report.required_completed_steps,
            report.optional_skipped_steps
        ),
        (1, 1, 1)
    );
    assert_eq!(report.software_result, "PASS");
    assert_eq!(report.hil_result, "NOT_REQUIRED");
    assert_eq!(report.hil_required_steps, 0);
    db.execute(
        "UPDATE learning_attempts SET state='completed' WHERE step_id=?1",
        [&optional.step_id],
    )
    .unwrap();
    assert_eq!(
        generate_local_report(&db, "r")
            .unwrap()
            .optional_completed_steps,
        1
    );
}

#[test]
fn optional_practice_can_start_without_binding_but_execution_still_requires_it() {
    let (mut db, session) = database(false);
    let (value, _) = manifest_and_lab(&db, &session).unwrap();
    assert!(!theory::entry_requires_project(&value));
    assert!(theory::entry_requires_project(&manifest(6, Some(true))));
    transition_step(&mut db, "r", "checking_understanding", None, None).unwrap();
    assert_eq!(
        transition_step(&mut db, "r", "practicing", Some("answer".into()), None).unwrap_err(),
        "LEARNING_PROJECT_REQUIRED"
    );
    let work = Connection::open_in_memory().unwrap();
    crate::workbench::storage::migrate(&work).unwrap();
    work.execute("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'test','127.0.0.1',22,'student','password')",[]).unwrap();
    work.execute("INSERT INTO projects(id,name,host_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'test',1,'/sdk','true','','')",[]).unwrap();
    for version in [5, 6] {
        db.execute("UPDATE learning_sessions SET project_id=0", [])
            .unwrap();
        let mut updated = value.clone();
        updated.schema_version = version;
        updated.labs[0].required = version == 5;
        db.execute(
            "UPDATE local_courses SET manifest_json=?1",
            [serde_json::to_string(&updated).unwrap()],
        )
        .unwrap();
        assert_eq!(
            theory::bind_project(&db, &work, "r", 1).unwrap().project_id,
            1
        );
    }
    transition_step(&mut db, "r", "practicing", Some("answer".into()), None).unwrap();
}

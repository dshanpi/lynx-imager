use super::*;

fn graph() -> LocalCourseManifest {
    let mut m = tests::manifest_v2_with_hil();
    m.schema_version = 8;
    m.labs.truncate(3);
    for lab in &mut m.labs {
        lab.hil_verification = None;
        lab.risk_level = "software-safe".into();
    }
    m.workspace = Some(LearnWorkspaceSpec {
        remote_subdir: "lesson".into(),
        package_source_root: "workspace".into(),
    });
    let mut value = serde_json::to_value(m).unwrap();
    value["labs"][0]["prerequisites"] = serde_json::json!(["step-3"]);
    value["labs"][1]["prerequisites"] = serde_json::json!([]);
    value["labs"][2]["prerequisites"] = serde_json::json!([]);
    serde_json::from_value(value).unwrap()
}

#[test]
fn prerequisites_schema_accepts_non_linear_order_and_rejects_ignored_legacy_edges() {
    let mut m = graph();
    assert!(
        validate_manifest(&m).is_ok(),
        "valid graph with a later-listed prerequisite"
    );
    m.schema_version = 7;
    assert!(
        validate_manifest(&m).is_err(),
        "legacy readers must not silently ignore gates"
    );
}

fn database() -> (Connection, LocalLearningSession) {
    let (db, _) = report_tests::database();
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&graph()).unwrap()],
    )
    .unwrap();
    db.execute(
        "UPDATE learning_sessions SET current_step='step-2',state='reading'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-2',1,'reading',1,1)", []).unwrap();
    let session = get_session(&db, "r").unwrap();
    (db, session)
}

fn request(session: &LocalLearningSession, target: &str, attempt: i64) -> revisit::Request {
    serde_json::from_value(serde_json::json!({"sessionId":session.session_id,"expectedStepId":session.current_step,"expectedAttempt":1,"expectedState":session.state,"targetStepId":target,"expectedTargetAttempt":attempt})).unwrap()
}

#[test]
fn prerequisites_independent_unvisited_activity_can_start_without_finishing_current() {
    let (mut db, s) = database();
    let selected = revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&s, "step-3", 0),
    )
    .unwrap();
    assert_eq!(selected.current_step, "step-3");
    assert_eq!(selected.state, "reading");
    assert_eq!(current_attempt(&db, "r", "step-3").unwrap(), 1);
    assert_eq!(
        revisit::latest(&db, "r", "step-2").unwrap().unwrap().1,
        "reading"
    );
}

#[test]
fn prerequisites_invalid_references_cycles_duplicates_and_missing_contract_are_rejected() {
    for bad in [
        serde_json::json!(["step-1"]),
        serde_json::json!(["missing"]),
        serde_json::json!(["step-3", "step-3"]),
        serde_json::Value::Null,
    ] {
        let mut value = serde_json::to_value(graph()).unwrap();
        value["labs"][0]["prerequisites"] = bad;
        let m: LocalCourseManifest = serde_json::from_value(value).unwrap();
        assert!(validate_manifest(&m).is_err());
    }
    let mut m = graph();
    m.labs[2].prerequisites = Some(vec!["step-1".into()]);
    assert_eq!(
        validate_manifest(&m).unwrap_err(),
        "LOCAL_COURSE_PREREQUISITES_CYCLE"
    );
    m = graph();
    m.labs[0].prerequisites = Some(vec!["step-3".into(); 512]);
    assert!(validate_manifest(&m).is_err());
    m = graph();
    m.schema_version = 9;
    assert!(validate_manifest(&m).is_err());
    assert_eq!(prerequisites::entry(&graph()).unwrap().step_id, "step-2");
}

fn finish(db: &Connection, id: &str, status: &str) {
    if revisit::latest(db, "r", id).unwrap().is_none() {
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r',?1,1,'completed',1,1)",[id]).unwrap();
    } else {
        db.execute("UPDATE learning_attempts SET state='completed' WHERE session_id='r' AND step_id=?1 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id='r' AND step_id=?1)",[id]).unwrap();
    }
    let mut at = get_session(db, "r").unwrap();
    at.current_step = id.into();
    append_evidence(
        db,
        &at,
        "command",
        "software",
        status,
        "synthetic unit fixture",
        &serde_json::json!({"workspaceSnapshot":snapshot()}),
    )
    .unwrap();
}
fn snapshot() -> freshness::Snapshot {
    serde_json::from_value(serde_json::json!({"version":3,"bindingSha256":"b".repeat(64),"rootSha256":"c".repeat(64),"treeSha256":"d".repeat(64),"entries":1,"bytes":1,"inputsSha256":"a".repeat(64)})).unwrap()
}
fn replace_manifest(db: &Connection, m: &LocalCourseManifest) {
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(m).unwrap()],
    )
    .unwrap();
}

#[test]
fn prerequisites_latest_attempt_skipped_and_missing_evidence_block_without_writes() {
    let (mut db, s) = database();
    let before = db.total_changes();
    assert!(revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&s, "step-1", 1)
    )
    .unwrap_err()
    .starts_with("LEARNING_PREREQUISITES_REQUIRED"));
    assert_eq!(db.total_changes(), before);
    finish(&db, "step-3", "PASS");
    prerequisites::require(&db, &s, &graph(), "step-1").unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-3',2,'reading',2,2)",[]).unwrap();
    assert!(prerequisites::require(&db, &s, &graph(), "step-1").is_err());
    db.execute(
        "UPDATE learning_attempts SET state='skipped' WHERE step_id='step-3' AND attempt=2",
        [],
    )
    .unwrap();
    assert_eq!(
        prerequisites::evaluate(&db, &s, &graph()).unwrap()[0].prerequisites[0].status,
        "skipped"
    );
    db.execute(
        "UPDATE learning_attempts SET state='completed' WHERE step_id='step-3' AND attempt=2",
        [],
    )
    .unwrap();
    assert_eq!(
        prerequisites::evaluate(&db, &s, &graph()).unwrap()[0].prerequisites[0].status,
        "verification_required"
    );
    finish(&db, "step-3", "FAIL");
    assert!(prerequisites::require(&db, &s, &graph(), "step-1").is_err());
    finish(&db, "step-3", "PASS");
    prerequisites::require(&db, &s, &graph(), "step-1").unwrap();
}

#[test]
fn prerequisites_transitive_optional_dependency_and_report_follow_current_facts() {
    let (db, s) = database();
    let mut m = graph();
    m.labs[0].prerequisites = Some(vec!["step-2".into()]);
    m.labs[1].prerequisites = Some(vec!["step-3".into()]);
    m.labs[2].required = false;
    replace_manifest(&db, &m);
    finish(&db, "step-2", "PASS");
    let access = prerequisites::evaluate(&db, &s, &m).unwrap();
    assert!(!access[0].available);
    assert_eq!(access[0].prerequisites.len(), 2);
    assert_eq!(access[0].prerequisites[0].status, "prerequisite_blocked");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.required_completed_steps, 0);
    assert_eq!(report.software_result, "INCOMPLETE");
    finish(&db, "step-3", "PASS");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.required_completed_steps, 2);
    assert_eq!(report.software_result, "PASS");
    assert_eq!(prerequisites::required_scope(&m).len(), 3);
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-3',2,'reading',2,2)",[]).unwrap();
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.required_completed_steps, 0);
    assert_eq!(report.software_result, "INCOMPLETE");
    assert_eq!(
        revisit::latest(&db, "r", "step-1").unwrap().unwrap().1,
        "completed"
    );
}

#[test]
fn prerequisites_advance_skips_blocked_order_then_returns_to_newly_available_activity() {
    let (db, s) = database();
    // Remove the pre-existing completed fixture so step-1 is genuinely unvisited.
    db.execute("DELETE FROM learning_evidence WHERE step_id='step-1'", [])
        .unwrap();
    db.execute("DELETE FROM learning_attempts WHERE step_id='step-1'", [])
        .unwrap();
    finish(&db, "step-2", "PASS");
    optional::advance(&db, &s).unwrap();
    let current = get_session(&db, "r").unwrap();
    assert_eq!(current.current_step, "step-3");
    finish(&db, "step-3", "PASS");
    optional::advance(&db, &current).unwrap();
    assert_eq!(get_session(&db, "r").unwrap().current_step, "step-1");
    assert_eq!(current_attempt(&db, "r", "step-1").unwrap(), 1);
}

#[test]
fn prerequisites_no_available_next_preserves_decision_and_reports_incomplete() {
    let (mut db, s) = database();
    let mut m = graph();
    m.labs[2].required = false;
    replace_manifest(&db, &m);
    db.execute("DELETE FROM learning_evidence WHERE step_id='step-1'", [])
        .unwrap();
    db.execute("DELETE FROM learning_attempts WHERE step_id='step-1'", [])
        .unwrap();
    finish(&db, "step-2", "PASS");
    optional::advance(&db, &s).unwrap();
    let at = get_session(&db, "r").unwrap();
    optional::skip_for_test(&mut db, &execution::ActionGate::default(), &at, 1).unwrap();
    let at = get_session(&db, "r").unwrap();
    assert_eq!(at.classification, "INCOMPLETE");
    assert_eq!(
        revisit::latest(&db, "r", "step-3").unwrap().unwrap().1,
        "skipped"
    );
    assert!(generate_local_report(&db, "r")
        .unwrap()
        .prerequisite_checks
        .iter()
        .any(|a| a.step_id == "step-1" && !a.available));
    let retried = revisit::select(
        &mut db,
        &execution::ActionGate::default(),
        request(&at, "step-3", 1),
    )
    .unwrap();
    assert_eq!(retried.state, "reading");
}

#[test]
fn prerequisites_new_selection_is_cas_guarded_busy_and_transactional() {
    let (mut db, s) = database();
    let actions = execution::ActionGate::default();
    let lease = actions.acquire(s.project_id).unwrap();
    assert!(revisit::select(&mut db, &actions, request(&s, "step-3", 0))
        .unwrap_err()
        .contains("BUSY"));
    drop(lease);
    db.execute_batch("CREATE TRIGGER fail_navigation BEFORE UPDATE ON learning_sessions BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get(0))
        .unwrap();
    assert!(revisit::select(&mut db, &actions, request(&s, "step-3", 0)).is_err());
    assert!(revisit::latest(&db, "r", "step-3").unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        count
    );
    db.execute_batch("DROP TRIGGER fail_navigation").unwrap();
    revisit::select(&mut db, &actions, request(&s, "step-3", 0)).unwrap();
    assert!(revisit::select(&mut db, &actions, request(&s, "step-3", 0)).is_err());
    assert_eq!(current_attempt(&db, "r", "step-3").unwrap(), 1);
}

#[test]
fn prerequisites_execution_plan_hash_checks_and_includes_optional_ancestors() {
    let (db, s) = database();
    let mut m = graph();
    m.labs[2].required = false;
    replace_manifest(&db, &m);
    finish(&db, "step-3", "PASS");
    let plans = prerequisites::execution_plan(&db, &s, &m, &m.labs[0]).unwrap();
    assert_eq!(plans.len(), 1);
    db.execute(
        "UPDATE learning_evidence SET detail_json='{}' WHERE step_id='step-3'",
        [],
    )
    .unwrap();
    assert!(prerequisites::execution_plan(&db, &s, &m, &m.labs[0]).is_err());
}

#[test]
fn prerequisites_input_changes_and_failed_observation_reject_before_execution() {
    let plans = vec![
        prerequisites::Input {
            snapshot: snapshot(),
            specs: vec![]
        };
        2
    ];
    let mut reads = 0;
    prerequisites::check_with(&plans, &"a".repeat(64), &"b".repeat(64), |_| {
        reads += 1;
        Ok(None)
    })
    .unwrap();
    assert_eq!(reads, 1);
    for (inputs, bound) in [("e", "b"), ("a", "e")] {
        let mut reads = 0;
        let result =
            prerequisites::check_with(&plans, &inputs.repeat(64), &bound.repeat(64), |_| {
                reads += 1;
                Ok(None)
            });
        assert!(result
            .unwrap_err()
            .starts_with("LEARNING_PREREQUISITE_INPUTS_CHANGED"));
        assert_eq!(reads, 0);
    }
    assert_eq!(
        prerequisites::check_with(&plans, &"a".repeat(64), &"b".repeat(64), |_| Err(
            "probe unavailable".into()
        ))
        .unwrap_err(),
        "probe unavailable"
    );
}

#[test]
fn prerequisites_hil_manual_pass_and_other_session_do_not_unlock() {
    let (db, s) = database();
    let mut m = graph();
    m.labs[2].hil_verification = tests::manifest_v2_with_hil().labs[1]
        .hil_verification
        .clone();
    m.labs[2].risk_level = "hardware-confirmation".into();
    replace_manifest(&db, &m);
    finish(&db, "step-3", "PASS");
    let mut at = s.clone();
    at.current_step = "step-3".into();
    append_evidence(
        &db,
        &at,
        "hil-verification",
        "hil-validator",
        "PASS",
        "legacy synthetic",
        &serde_json::json!({}),
    )
    .unwrap();
    assert!(prerequisites::require(&db, &s, &m, "step-1").is_err());
    db.execute(
        "UPDATE learning_attempts SET session_id='foreign' WHERE step_id='step-3'",
        [],
    )
    .unwrap();
    assert_eq!(
        prerequisites::evaluate(&db, &s, &m).unwrap()[0].prerequisites[0].status,
        "not_started"
    );
}

fn reflection(lab: &mut LocalLab) {
    lab.activity_kind = "reflection".into();
    lab.verification_rules.clear();
    lab.required_evidence.clear();
    lab.hil_verification = None;
    lab.risk_level = "reading-safe".into();
    lab.document = Some(LearnDocumentSpec {
        path: format!("docs/{}.md", lab.step_id),
        sha256: "a".repeat(64),
        source_url: "https://example.invalid/course".into(),
    });
}
#[test]
fn prerequisites_first_available_theory_root_does_not_require_project() {
    let mut m = graph();
    reflection(&mut m.labs[1]);
    validate_manifest(&m).unwrap();
    assert_eq!(prerequisites::entry(&m).unwrap().step_id, "step-2");
    assert!(!theory::entry_requires_project(&m));
    m.schema_version = 7;
    for l in &mut m.labs {
        l.prerequisites = None;
    }
    assert!(theory::entry_requires_project(&m));
}
#[test]
fn prerequisites_optional_practice_is_verified_even_when_only_required_activity_is_theory() {
    let (db, _) = database();
    let mut m = graph();
    reflection(&mut m.labs[0]);
    m.labs[1].required = false;
    m.labs[2].required = false;
    replace_manifest(&db, &m);
    db.execute(
        "UPDATE learning_attempts SET answer='reflection answer' WHERE step_id='step-1'",
        [],
    )
    .unwrap();
    finish(&db, "step-3", "PASS");
    let s = get_session(&db, "r").unwrap();
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(report.software_result, "PASS");
    assert_eq!(report.required_completed_steps, 1);
    let plans = revisit::source_plan(&db, &s).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].step_id, "step-3");
    let legacy = serde_json::json!({"workspaceSnapshot":{"version":1,"bindingSha256":"b".repeat(64),"rootSha256":"c".repeat(64),"treeSha256":"d".repeat(64),"entries":1,"bytes":1}});
    let mut at = s.clone();
    at.current_step = "step-3".into();
    append_evidence(
        &db,
        &at,
        "command",
        "software",
        "PASS",
        "legacy fixture",
        &legacy,
    )
    .unwrap();
    assert!(prerequisites::execution_plan(&db, &s, &m, &m.labs[0])
        .unwrap_err()
        .starts_with("LEARNING_INPUTS_UNRECORDED"));
}
#[test]
fn prerequisites_environment_receipt_changes_reject_even_with_identical_sources() {
    let specs:Vec<environment::CheckSpec>=serde_json::from_value(serde_json::json!([{"id":"cc","title":"Compiler","kind":"tool","selector":"cc","operator":"present","expected":"","remediation":"restore compiler"}])).unwrap();
    let receipt = |version: &str| {
        let observation = serde_json::json!({"source":{"kind":"directory","commit":"","branch":"","dirty":null,"untracked":null,"status":"","message":""},"host":{"os":"Ubuntu","architecture":"x86_64"},"tools":[{"name":"cc","status":"OBSERVED","path":"/usr/bin/cc","version":version,"sha256":"b".repeat(64),"message":""}],"checks":[{"spec":specs[0],"observed":version,"status":"MATCH","message":""}]});
        Some(environment_receipt::seal(&serde_json::to_vec(&observation).unwrap(), &specs).unwrap())
    };
    let original = receipt("cc one");
    let mut saved = snapshot();
    saved.environment = original.clone();
    let plans = vec![prerequisites::Input {
        snapshot: saved,
        specs: specs.clone(),
    }];
    prerequisites::check_with(&plans, &"a".repeat(64), &"b".repeat(64), |_| {
        Ok(original.clone())
    })
    .unwrap();
    assert!(
        prerequisites::check_with(&plans, &"a".repeat(64), &"b".repeat(64), |_| Ok(receipt(
            "cc two"
        )))
        .unwrap_err()
        .starts_with("LEARNING_PREREQUISITE_INPUTS_CHANGED")
    );
}

#[test]
fn prerequisites_optional_hil_ancestor_requires_bound_device_but_unrelated_optional_does_not() {
    let work = Connection::open_in_memory().unwrap();
    crate::workbench::storage::migrate(&work).unwrap();
    work.execute("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'test','127.0.0.1',22,'student','password')",[]).unwrap();
    work.execute("INSERT INTO projects(id,name,host_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'test',1,'/sdk','true','','')",[]).unwrap();
    let mut m = graph();
    m.labs[2].required = false;
    m.labs[2].hil_verification = tests::manifest_v2_with_hil()
        .labs
        .iter()
        .find_map(|l| l.hil_verification.clone());
    assert!(m.labs[2].hil_verification.is_some());
    assert_eq!(
        theory::project_identity(&work, &m, 1).unwrap_err(),
        "LEARNING_DEVICE_REQUIRED"
    );
    m.labs[0].prerequisites = Some(vec![]);
    assert_eq!(
        theory::project_identity(&work, &m, 1).unwrap(),
        (None, String::new())
    );
}

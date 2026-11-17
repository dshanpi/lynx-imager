use super::*;

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let mut m = tests::manifest_v2_with_hil();
    m.schema_version = 3;
    m.labs = vec![m.labs[1].clone()];
    m.labs[0].number = 1;
    m.workspace = Some(LearnWorkspaceSpec {
        remote_subdir: "lesson".into(),
        package_source_root: "workspace".into(),
    });
    validate_manifest(&m).unwrap();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','HIL','board','signer','hash',?1,'/unused',1)",[serde_json::to_string(&m).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,classification,device_id,board_uid,created_at,updated_at) VALUES('hil','t527-course','1.0.0-rc.1',1,?1,'practicing','INCOMPLETE',1,'board',1,1)",[&m.labs[0].step_id]).unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('hil',?1,1,'practicing',1,1)",[&m.labs[0].step_id]).unwrap();
    let session = get_session(&db, "hil").unwrap();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "tool exit zero",
        &serde_json::json!({}),
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "wiring-observation",
        "hil-device",
        "PASS",
        "user pasted observation",
        &serde_json::json!({"observation":"looks fine"}),
    )
    .unwrap();
    append_evidence(
        &db,
        &session,
        "hil-verification",
        "hil-validator",
        "PASS",
        "old manual aggregate",
        &serde_json::json!({"actionId":"verify-wiring"}),
    )
    .unwrap();
    db
}

#[test]
fn hil_manual_legacy_pass_cannot_advance_hardware_step() {
    let db = database();
    let session = get_session(&db, "hil").unwrap();
    assert!(require_current_verification(&db, &session).is_err());
}

#[test]
fn hil_manual_legacy_pass_cannot_count_as_hardware_report_pass() {
    let db = database();
    db.execute("UPDATE learning_attempts SET state='completed'", [])
        .unwrap();
    db.execute(
        "UPDATE learning_sessions SET state='completed',classification='FULL_PASS'",
        [],
    )
    .unwrap();
    let report = generate_local_report(&db, "hil").unwrap();
    assert_eq!(report.hil_result, "NOT_VERIFIED");
    assert_eq!(report.hil_passed_steps, 0);
    assert_eq!(report.recorded_hil_result, "PASS");
    assert_eq!(report.session.classification, "HIL_NOT_VERIFIED");
    assert_eq!(report.session.recorded_classification, "FULL_PASS");
}

fn approval(db: &Connection) {
    let session = get_session(db, "hil").unwrap();
    let (_, lab) = manifest_and_lab(db, &session).unwrap();
    db.execute("INSERT INTO learning_hil_approvals VALUES('test-token','hil',?1,1,?2,1,'board','{}',9999999999,1)",params![session.current_step,lab.hil_verification.unwrap().action_id]).unwrap();
}
fn request(db: &Connection) -> HilEvidenceRequest {
    serde_json::from_value(serde_json::json!({"sessionId":"hil","expectedStepId":get_session(db,"hil").unwrap().current_step,"expectedAttempt":1,"evidenceKind":"wiring-observation","status":"PASS","summary":"password=secret-demo-value","observation":"password=secret-demo-value","durationSeconds":7200,"artifactSha256":"a".repeat(64)})).unwrap()
}
fn count(db: &Connection) -> i64 {
    db.query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn hil_claimed_pass_hash_and_duration_are_only_user_observations() {
    let mut db = database();
    approval(&db);
    let actions = execution::ActionGate::default();
    let result = attach_hil_observation(&db, &actions, request(&db)).unwrap();
    assert_eq!(result.source, "user-asserted");
    assert_eq!(result.status, "RECORDED");
    assert!(!result.summary.contains("secret-demo-value"));
    let detail: String = db
        .query_row(
            "SELECT detail_json FROM learning_evidence ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!detail.contains("secret-demo-value"));
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(detail["claimedStatus"], "PASS");
    assert_eq!(detail["durationSeconds"], 7200);
    let assessment = hil_trust::assessment(&mut db, &actions, "hil").unwrap();
    assert_eq!(assessment.status, "NOT_VERIFIED");
    assert_eq!(assessment.source, "hil-assessment");
    assert_eq!(get_session(&db, "hil").unwrap().state, "practicing");
    assert!(require_current_verification(&db, &get_session(&db, "hil").unwrap()).is_err());
}

#[test]
fn hil_observation_rejects_stale_missing_approval_and_busy_without_writes() {
    let db = database();
    let actions = execution::ActionGate::default();
    let before = count(&db);
    assert_eq!(
        attach_hil_observation(&db, &actions, request(&db)).unwrap_err(),
        "LEARNING_HIL_APPROVAL_REQUIRED"
    );
    approval(&db);
    for mode in 0..3 {
        let mut req = request(&db);
        match mode {
            0 => req.expected_step_id = Some("other".into()),
            1 => req.expected_attempt = Some(2),
            _ => req.expected_attempt = None,
        }
        assert_eq!(
            attach_hil_observation(&db, &actions, req).unwrap_err(),
            "LEARNING_ACTION_CONTEXT_CHANGED"
        );
    }
    let lease = actions.acquire(1).unwrap();
    assert!(attach_hil_observation(&db, &actions, request(&db))
        .unwrap_err()
        .starts_with("LEARNING_ACTION_BUSY:"));
    assert_eq!(count(&db), before);
    drop(lease);
}

#[test]
fn hil_unknown_trust_fields_are_not_a_receipt() {
    let value = serde_json::json!({"sessionId":"hil","expectedStepId":"s","expectedAttempt":1,"evidenceKind":"wiring-observation","status":"PASS","summary":"claim","observation":"claim","source":"hil-device","verified":true});
    assert!(serde_json::from_value::<HilEvidenceRequest>(value).is_err());
}

#[test]
fn hil_legacy_projection_retains_original_records_without_mutation() {
    let db = database();
    let before: String = db
        .query_row(
            "SELECT group_concat(source||':'||status||':'||detail_json,'|') FROM learning_evidence",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let session = get_session(&db, "hil").unwrap();
    let projected = list_step_evidence(&db, "hil", &session.current_step).unwrap();
    let json = serde_json::to_value(&projected).unwrap();
    assert_eq!(json[0]["status"], "NOT_VERIFIED");
    assert_eq!(json[0]["recorded"]["status"], "PASS");
    assert_eq!(json[1]["source"], "user-asserted");
    assert_eq!(json[1]["status"], "RECORDED");
    assert_eq!(json[1]["recorded"]["source"], "hil-device");
    let after: String = db
        .query_row(
            "SELECT group_concat(source||':'||status||':'||detail_json,'|') FROM learning_evidence",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}

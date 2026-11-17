use super::*;

fn database() -> (Connection, LocalLearningSession) {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let manifest = tests::manifest_v2_with_hil();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','T527','board','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('tutor','t527-course','1.0.0-rc.1',1,?1,'practicing',1,1)", [&manifest.labs[1].step_id]).unwrap();
    let session = get_session(&db, "tutor").unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('tutor',?1,1,'practicing',1,1)", [&session.current_step]).unwrap();
    (db, session)
}

fn request(session: &LocalLearningSession, digest: Option<String>) -> LocalTutorRequest {
    LocalTutorRequest {
        session_id: session.session_id.clone(),
        question: "为什么编码失败？".into(),
        hint_level: "L2".into(),
        expected_step_id: Some(session.current_step.clone()),
        expected_attempt: Some(1),
        diagnostic_digest: digest,
    }
}

#[test]
fn local_tutor_diagnostic_is_opt_in_and_read_only() {
    let (mut db, session) = database();
    let result = record_action_result(
        &mut db,
        &session,
        1,
        1,
        0,
        b"Unknown encoder synthetic-fixture",
        true,
    )
    .unwrap();
    let before = db.total_changes();
    let (_, _, ordinary) = tutor_context::prepare(&db, &request(&session, None)).unwrap();
    assert!(ordinary.get("commandDiagnostic").is_none());
    assert!(!ordinary.to_string().contains("synthetic-fixture"));
    let (_, _, context) =
        tutor_context::prepare(&db, &request(&session, Some(result.output_digest.clone())))
            .unwrap();
    let detail = &context["commandDiagnostic"];
    assert_eq!(detail["digest"], result.output_digest);
    assert_eq!(detail["text"], "Unknown encoder synthetic-fixture");
    assert_eq!(detail["source"], "software");
    assert_eq!(detail["trust"], "client_observation");
    assert_eq!(detail["artifactStateVerified"], false);
    assert_eq!(detail["hardwareVerified"], false);
    assert_eq!(detail["rawOutputIndependentlyVerified"], false);
    assert_eq!(detail["exitCode"], 1);
    assert_eq!(detail["expectedExitCode"], 0);
    assert!(detail["createdAt"].as_i64().unwrap() > 0);
    assert_eq!(db.total_changes(), before);
}

#[test]
fn local_tutor_diagnostic_requires_explicit_expected_scope() {
    let (mut db, session) = database();
    let result = record_action_result(&mut db, &session, 1, 1, 0, b"error", true).unwrap();
    for mode in [
        "missing-step",
        "missing-attempt",
        "wrong-step",
        "wrong-attempt",
        "unknown-digest",
    ] {
        let mut req = request(&session, Some(result.output_digest.clone()));
        match mode {
            "missing-step" => req.expected_step_id = None,
            "missing-attempt" => req.expected_attempt = None,
            "wrong-step" => req.expected_step_id = Some("other".into()),
            "wrong-attempt" => req.expected_attempt = Some(2),
            _ => req.diagnostic_digest = Some("0".repeat(64)),
        }
        assert!(tutor_context::prepare(&db, &req).is_err(), "{mode}");
    }
}

#[test]
fn local_tutor_legacy_and_other_attempt_diagnostics_are_rejected() {
    let (db, session) = database();
    let digest = append_evidence(
        &db,
        &session,
        "command",
        "software",
        "FAIL",
        "old",
        &serde_json::json!({"outputTail":"old raw tail"}),
    )
    .unwrap();
    assert!(tutor_context::prepare(&db, &request(&session, Some(digest))).is_err());
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('tutor',?1,2,'practicing',2,2)", [&session.current_step]).unwrap();
    assert!(tutor_context::prepare(&db, &request(&session, None)).is_err());
}

#[test]
fn local_tutor_diagnostic_is_bounded_redacted_and_marked_as_truncated() {
    let (mut db, session) = database();
    let output = format!("api_key=synthetic-secret\n{}\nlast error", "x".repeat(4000));
    let result = record_action_result(&mut db, &session, 1, 1, 0, output.as_bytes(), true).unwrap();
    let (_, _, context) =
        tutor_context::prepare(&db, &request(&session, Some(result.output_digest))).unwrap();
    let detail = &context["commandDiagnostic"];
    assert_eq!(detail["truncated"], true);
    let text = detail["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), 2048);
    assert!(text.ends_with("last error"));
    assert!(!context.to_string().contains("synthetic-secret"));
}

use super::*;

fn database() -> (Connection, LocalLearningSession) {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let manifest = tests::manifest_v2_with_hil();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','T527','board','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('diag','t527-course','1.0.0-rc.1',1,?1,'practicing',1,1)", [&manifest.labs[1].step_id]).unwrap();
    let session = get_session(&db, "diag").unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('diag',?1,1,'practicing',1,1)", [&session.current_step]).unwrap();
    (db, session)
}

fn request(session: &LocalLearningSession, digest: &str) -> diagnostics::DiagnosticRequest {
    diagnostics::DiagnosticRequest {
        session_id: session.session_id.clone(),
        step_id: session.current_step.clone(),
        attempt: 1,
        digest: digest.into(),
    }
}

#[test]
fn diagnostics_actual_failure_success_and_history_are_read_only() {
    let (mut db, session) = database();
    let failure = record_action_result(
        &mut db,
        &session,
        1,
        1,
        0,
        b"Unknown encoder: example",
        true,
    )
    .unwrap();
    let pass = record_action_result(&mut db, &session, 1, 0, 0, b"sampleRate=16000", true).unwrap();
    let before = db.total_changes();
    let old = diagnostics::read(&db, &request(&session, &failure.output_digest)).unwrap();
    assert_eq!(old.status, "FAIL");
    assert_eq!(old.exit_code, Some(1));
    assert_eq!(old.expected_exit_code, Some(0));
    assert_eq!(old.text.as_deref(), Some("Unknown encoder: example"));
    let current = diagnostics::read(&db, &request(&session, &pass.output_digest)).unwrap();
    assert_eq!(current.text.as_deref(), Some("sampleRate=16000"));
    assert_eq!(current.availability, "available");
    assert!(!current.truncated);
    assert_eq!(db.total_changes(), before);
}

#[test]
fn diagnostics_scope_and_noncommand_records_are_rejected() {
    let (mut db, session) = database();
    let result = record_action_result(&mut db, &session, 1, 0, 0, b"ok", true).unwrap();
    for field in ["session", "step", "attempt", "digest"] {
        let mut req = request(&session, &result.output_digest);
        match field {
            "session" => req.session_id = "other".into(),
            "step" => req.step_id = "other".into(),
            "attempt" => req.attempt = 2,
            _ => req.digest = "a".repeat(64),
        }
        assert!(diagnostics::read(&db, &req).is_err());
    }
    let digest = append_evidence(
        &db,
        &session,
        "photo",
        "user-camera",
        "RECORDED",
        "photo",
        &serde_json::json!({"secret":"must not expose"}),
    )
    .unwrap();
    assert!(diagnostics::read(&db, &request(&session, &digest)).is_err());
}

#[test]
fn diagnostics_hash_tampering_and_oversize_are_rejected() {
    let (mut db, session) = database();
    let result = record_action_result(&mut db, &session, 1, 0, 0, b"ok", true).unwrap();
    db.execute("UPDATE learning_evidence SET detail_json='{}'", [])
        .unwrap();
    assert!(diagnostics::read(&db, &request(&session, &result.output_digest)).is_err());
    let digest = append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "large",
        &serde_json::json!({"padding":"x".repeat(70000)}),
    )
    .unwrap();
    assert!(diagnostics::read(&db, &request(&session, &digest)).is_err());
}

#[test]
fn diagnostics_old_format_is_explicitly_unavailable_not_empty() {
    let (db, session) = database();
    let digest = append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "old",
        &serde_json::json!({"outputTail":"legacy unverified output"}),
    )
    .unwrap();
    let result = diagnostics::read(&db, &request(&session, &digest)).unwrap();
    assert_eq!(result.availability, "legacy_unavailable");
    assert_eq!(result.text, None);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("legacy unverified output"));
}

#[test]
fn diagnostics_redacts_before_tail_boundary_and_records_truncation() {
    let (mut db, session) = database();
    let input = format!("password={}\nlast diagnostic", "S".repeat(4000));
    let result = record_action_result(&mut db, &session, 1, 1, 0, input.as_bytes(), true).unwrap();
    let view = diagnostics::read(&db, &request(&session, &result.output_digest)).unwrap();
    let text = view.text.unwrap();
    assert!(!text.contains("SSSS"));
    assert!(text.ends_with("last diagnostic"));
    let input = format!("{}\nlast diagnostic", "界".repeat(4000));
    let result = record_action_result(&mut db, &session, 1, 1, 0, input.as_bytes(), true).unwrap();
    let view = diagnostics::read(&db, &request(&session, &result.output_digest)).unwrap();
    assert!(view.truncated);
    assert_eq!(view.text.unwrap().chars().count(), 2048);
}

#[test]
fn diagnostics_error_and_empty_output_are_distinct() {
    let (mut db, session) = database();
    let result = record_action_result(&mut db, &session, 1, 0, 0, b"", true).unwrap();
    assert_eq!(
        diagnostics::read(&db, &request(&session, &result.output_digest))
            .unwrap()
            .text
            .as_deref(),
        Some("")
    );
    record_action_failure(&db, &session, 1, "LEARNING_ACTION_TIMEOUT").unwrap();
    let digest: String = db
        .query_row(
            "SELECT digest FROM learning_evidence ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let view = diagnostics::read(&db, &request(&session, &digest)).unwrap();
    assert_eq!(view.status, "ERROR");
    assert_eq!(view.exit_code, None);
    assert_eq!(view.text.as_deref(), Some("LEARNING_ACTION_TIMEOUT"));
}

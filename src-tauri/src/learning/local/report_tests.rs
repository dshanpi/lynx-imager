use super::*;

pub(super) fn database() -> (Connection, LocalLearningSession) {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let mut manifest = tests::manifest_v2_with_hil();
    manifest.labs.truncate(1);
    manifest.labs[0].hil_verification = None;
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','Fixture','board','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('r','t527-course','1.0.0-rc.1',1,'step-1','completed',1,1)", []).unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-1',1,'completed',1,1)", []).unwrap();
    let session = get_session(&db, "r").unwrap();
    append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "historical",
        &serde_json::json!({}),
    )
    .unwrap();
    (db, session)
}

#[test]
fn historical_report_explicitly_marks_workspace_not_yet_checked() {
    let (db, _) = database();
    let report = serde_json::to_value(generate_local_report(&db, "r").unwrap()).unwrap();
    assert_eq!(report["recordedSoftwareResult"], "PASS");
    assert_eq!(report["currentWorkspace"]["status"], "NOT_CHECKED");
}

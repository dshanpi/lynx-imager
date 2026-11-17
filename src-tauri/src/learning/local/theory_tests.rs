use super::*;

fn manifest() -> LocalCourseManifest {
    let mut value = serde_json::to_value(super::tests::manifest_v2_with_hil()).unwrap();
    value["schemaVersion"] = 4.into();
    value["workspace"] = serde_json::Value::Null;
    let mut lab = value["labs"][0].clone();
    lab["activityKind"] = "reflection".into();
    lab["riskLevel"] = "reading-safe".into();
    lab["verificationRules"] = serde_json::json!([]);
    lab["requiredEvidence"] = serde_json::json!([]);
    lab["hilVerification"] = serde_json::Value::Null;
    lab["document"] = serde_json::json!({"path":"docs/lesson.md","sha256":"a".repeat(64),"sourceUrl":"https://example.invalid/theory"});
    value["labs"] = serde_json::json!([lab]);
    serde_json::from_value(value).unwrap()
}

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let manifest = manifest();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','Theory','none','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('theory','t527-course','1.0.0-rc.1',0,?1,'reading',1,1)", [&manifest.labs[0].step_id]).unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('theory',?1,1,'reading',1,1)", [&manifest.labs[0].step_id]).unwrap();
    db
}

#[test]
fn theory_signed_manifest_needs_neither_workspace_nor_command() {
    validate_manifest(&manifest()).unwrap();
}

#[test]
fn theory_completion_requires_answer_and_reflection_without_fabricated_pass() {
    let mut db = database();
    assert!(transition_step(&mut db, "theory", "completed", None, Some("skip".into())).is_err());
    transition_step(&mut db, "theory", "checking_understanding", None, None).unwrap();
    assert!(transition_step(&mut db, "theory", "reflecting", Some(" ".into()), None).is_err());
    transition_step(
        &mut db,
        "theory",
        "reflecting",
        Some("My understanding".into()),
        None,
    )
    .unwrap();
    assert!(transition_step(&mut db, "theory", "completed", None, Some(" ".into())).is_err());
    let done = transition_step(
        &mut db,
        "theory",
        "completed",
        None,
        Some("Still need to review".into()),
    )
    .unwrap();
    assert_eq!(done.classification, "REFLECTION_COMPLETED");
    let report = generate_local_report(&db, "theory").unwrap();
    assert_eq!(report.completed_steps, 1);
    assert_eq!(report.software_result, "NOT_REQUIRED");
    assert_eq!(report.hil_result, "NOT_REQUIRED");
    let count: i64 = db.query_row("SELECT COUNT(*) FROM learning_evidence WHERE source IN ('software','hil-validator') OR status='PASS'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn theory_rejects_unknown_legacy_and_executable_activity_declarations() {
    let valid = manifest();
    for version in 1..=3 {
        let mut value = valid.clone();
        value.schema_version = version;
        assert!(validate_manifest(&value).is_err());
    }
    for kind in ["unknown", "Practice", ""] {
        let mut value = valid.clone();
        value.labs[0].activity_kind = kind.into();
        assert!(validate_manifest(&value).is_err());
    }
    let practical = super::tests::manifest_v2_with_hil();
    let mut value = valid.clone();
    value.labs[0].verification_rules = practical.labs[0].verification_rules.clone();
    assert!(validate_manifest(&value).is_err());
    let mut value = valid.clone();
    value.labs[0].hil_verification = practical.labs[1].hil_verification.clone();
    assert!(validate_manifest(&value).is_err());
    let mut value = valid.clone();
    value.labs[0].document = None;
    assert!(validate_manifest(&value).is_err());
    let mut value = valid;
    value.labs[0].required_evidence = practical.labs[0].required_evidence.clone();
    assert!(validate_manifest(&value).is_err());
}

#[test]
fn theory_cannot_enter_execution_or_bind_a_project() {
    let mut db = database();
    let work = Connection::open_in_memory().unwrap();
    assert!(theory::bind_project(&db, &work, "theory", 1).is_err());
    transition_step(&mut db, "theory", "checking_understanding", None, None).unwrap();
    for state in [
        "practicing",
        "collecting_evidence",
        "verifying",
        "completed",
    ] {
        assert!(transition_step(
            &mut db,
            "theory",
            state,
            Some("answer".into()),
            Some("reflection".into())
        )
        .is_err());
    }
    assert_eq!(get_session(&db, "theory").unwrap().project_id, 0);
    assert_eq!(
        generate_local_report(&db, "theory")
            .unwrap()
            .completed_steps,
        0
    );
}

#[test]
fn mixed_course_binds_only_before_practice_and_keeps_theory_records() {
    let mut db = database();
    let mut mixed = manifest();
    let mut practice = super::tests::manifest_v2_with_hil().labs[0].clone();
    practice.step_id = "practice".into();
    practice.number = 2;
    mixed.labs.push(practice);
    assert!(validate_manifest(&mixed).is_err());
    mixed.workspace = Some(LearnWorkspaceSpec {
        remote_subdir: "lesson".into(),
        package_source_root: "workspace".into(),
    });
    validate_manifest(&mixed).unwrap();
    assert!(!theory::entry_requires_project(&mixed));
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&mixed).unwrap()],
    )
    .unwrap();
    transition_step(&mut db, "theory", "checking_understanding", None, None).unwrap();
    transition_step(
        &mut db,
        "theory",
        "reflecting",
        Some("theory answer".into()),
        None,
    )
    .unwrap();
    transition_step(
        &mut db,
        "theory",
        "completed",
        None,
        Some("theory reflection".into()),
    )
    .unwrap();
    assert_eq!(get_session(&db, "theory").unwrap().current_step, "practice");
    transition_step(&mut db, "theory", "checking_understanding", None, None).unwrap();
    assert_eq!(
        transition_step(&mut db, "theory", "practicing", Some("answer".into()), None).unwrap_err(),
        "LEARNING_PROJECT_REQUIRED"
    );
    let work = Connection::open_in_memory().unwrap();
    crate::workbench::storage::migrate(&work).unwrap();
    work.execute("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'test','127.0.0.1',22,'student','password')",[]).unwrap();
    work.execute("INSERT INTO projects(id,name,host_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'test',1,'/sdk','true','','')",[]).unwrap();
    assert!(theory::bind_project(&db, &work, "theory", 0).is_err());
    assert!(theory::bind_project(&db, &work, "theory", 999).is_err());
    let bound = theory::bind_project(&db, &work, "theory", 1).unwrap();
    assert_eq!(bound.project_id, 1);
    assert!(theory::bind_project(&db, &work, "theory", 1).is_err());
    transition_step(&mut db, "theory", "practicing", Some("answer".into()), None).unwrap();
    assert!(transition_step(&mut db, "theory", "reflecting", Some("skip".into()), None).is_err());
    let report = generate_local_report(&db, "theory").unwrap();
    assert_eq!(report.completed_steps, 1);
    assert_eq!(report.software_result, "INCOMPLETE");
    let answer: String = db
        .query_row(
            "SELECT answer FROM learning_attempts WHERE step_id=?1",
            [&mixed.labs[0].step_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(answer, "theory answer");
}

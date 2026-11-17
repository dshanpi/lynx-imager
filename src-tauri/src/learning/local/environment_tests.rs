use super::*;

#[test]
fn environment_schema7_keeps_artifact_collection() {
    let mut m = configured(7);
    m.labs.truncate(1);
    m.labs[0].artifact_outputs = serde_json::from_value(serde_json::json!([{
        "artifactId":"metrics","title":"指标","path":"build/metrics.json",
        "mimeType":"application/json","maxBytes":65536
    }]))
    .unwrap();
    validate_manifest(&m).unwrap();
    m.labs[0].artifact_outputs[0].path = "../escape.json".into();
    assert!(validate_manifest(&m).is_err());
}

#[test]
fn environment_schema7_keeps_optional_skip_without_pass() {
    let (mut db, _) = report_tests::database();
    let mut m = configured(7);
    m.labs.truncate(1);
    m.labs[0].required = false;
    validate_manifest(&m).unwrap();
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&m).unwrap()],
    )
    .unwrap();
    db.execute("DELETE FROM learning_evidence", []).unwrap();
    db.execute(
        "UPDATE learning_sessions SET project_id=0,state='reading'",
        [],
    )
    .unwrap();
    db.execute("UPDATE learning_attempts SET state='reading'", [])
        .unwrap();
    let session = get_session(&db, "r").unwrap();
    let done =
        optional::skip_for_test(&mut db, &execution::ActionGate::default(), &session, 1).unwrap();
    assert_eq!(done.classification, "OPTIONAL_COMPLETED");
    let report = generate_local_report(&db, "r").unwrap();
    assert_eq!(
        (report.completed_steps, report.optional_skipped_steps),
        (0, 1)
    );
    assert_eq!(report.software_result, "NOT_REQUIRED");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM learning_evidence WHERE status='PASS'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn environment_schema7_keeps_deferred_project_binding() {
    let (db, _) = report_tests::database();
    let mut m = configured(7);
    m.labs.truncate(1);
    validate_manifest(&m).unwrap();
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&m).unwrap()],
    )
    .unwrap();
    db.execute(
        "UPDATE learning_sessions SET project_id=0,state='reading'",
        [],
    )
    .unwrap();
    let work = WorkbenchState::open(Path::new(":memory:")).unwrap();
    let work = work.db.lock().unwrap();
    work.execute_batch("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'fixture','127.0.0.1',22,'fixture','private_key'); INSERT INTO projects(id,name,host_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'fixture',1,'/sdk','','','');").unwrap();
    assert_eq!(
        theory::bind_project(&db, &work, "r", 1).unwrap().project_id,
        1
    );
    assert_eq!(
        theory::bind_project(&db, &work, "r", 1).unwrap_err(),
        "LEARNING_PROJECT_BINDING_NOT_ALLOWED"
    );
}

#[test]
fn environment_late_result_rejects_changed_binding_step_and_attempt() {
    let local = LocalLearningState::open(Path::new(":memory:"), std::env::temp_dir()).unwrap();
    let work = WorkbenchState::open(Path::new(":memory:")).unwrap();
    work.db.lock().unwrap().execute_batch("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'fixture','127.0.0.1',22,'fixture','private_key'); INSERT INTO projects(id,name,host_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'fixture',1,'/sdk','','','');").unwrap();
    local.db.lock().unwrap().execute_batch("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('inspect','course','1',1,'source','reading',1,1); INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('inspect','source',1,'reading',1,1);").unwrap();
    let session = get_session(&local.db.lock().unwrap(), "inspect").unwrap();
    let binding = freshness::current_binding(&work.db.lock().unwrap(), &session).unwrap();
    environment::ensure_context(&local, &work, &session, 1, &binding).unwrap();
    work.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET sdk_dir='/changed'", [])
        .unwrap();
    assert_eq!(
        environment::ensure_context(&local, &work, &session, 1, &binding).unwrap_err(),
        "LEARNING_INPUTS_CHANGED:工程绑定已变化，请重新检查"
    );
    work.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET sdk_dir='/sdk'", [])
        .unwrap();
    local
        .db
        .lock()
        .unwrap()
        .execute("UPDATE learning_sessions SET current_step='other'", [])
        .unwrap();
    assert_eq!(
        environment::ensure_context(&local, &work, &session, 1, &binding).unwrap_err(),
        "LEARNING_ACTION_CONTEXT_CHANGED"
    );
    local
        .db
        .lock()
        .unwrap()
        .execute("UPDATE learning_sessions SET current_step='source'", [])
        .unwrap();
    local.db.lock().unwrap().execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('inspect','source',2,'reading',1,1)",[]).unwrap();
    assert_eq!(
        environment::ensure_context(&local, &work, &session, 1, &binding).unwrap_err(),
        "LEARNING_ACTION_CONTEXT_CHANGED"
    );
}

fn configured(schema: u32) -> LocalCourseManifest {
    let mut value = serde_json::to_value(tests::manifest_v2_with_hil()).unwrap();
    value["schemaVersion"] = schema.into();
    value["workspace"] =
        serde_json::json!({"remoteSubdir":"lesson","packageSourceRoot":"workspace"});
    value["labs"][0]["environmentChecks"] = serde_json::json!([{
        "id":"compiler", "title":"系统 C 编译器", "kind":"tool", "selector":"cc",
        "operator":"present", "expected":"", "remediation":"请先安装课程所需 C 编译工具。"
    }]);
    serde_json::from_value(value).unwrap()
}

#[test]
fn environment_schema7_preserves_activity_requirements() {
    let manifest = configured(7);
    validate_manifest(&manifest).unwrap();
    let value = serde_json::to_value(manifest).unwrap();
    assert_eq!(value["labs"][0]["environmentChecks"][0]["selector"], "cc");
}

#[test]
fn environment_old_schema_cannot_silently_drop_requirements() {
    for schema in [2, 3, 4, 5, 6] {
        assert!(validate_manifest(&configured(schema)).is_err());
    }
}

#[test]
fn environment_declared_kinds_and_operators_are_bounded() {
    for (kind, selector, operator, expected) in [
        ("tool", "cc", "present", ""),
        ("tool", "cc", "eq", "cc 1"),
        (
            "tool",
            "cc",
            "sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        (
            "file",
            "sdk/VERSION",
            "sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        ("os", "", "eq", "Ubuntu"),
        ("architecture", "", "eq", "x86_64"),
        ("git", "", "eq", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        let mut m = configured(7);
        let spec = &mut m.labs[0].environment_checks[0];
        spec.kind = kind.into();
        spec.selector = selector.into();
        spec.operator = operator.into();
        spec.expected = expected.into();
        validate_manifest(&m).unwrap();
    }
    for (kind, selector, operator, expected) in [
        ("tool", "sh", "present", ""),
        ("tool", "cc --help", "present", ""),
        ("tool", "cc", "shell", "true"),
        ("tool", "cc", "sha256", "wrong"),
        (
            "file",
            "../secret",
            "sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        (
            "file",
            "C:\\secret",
            "sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        ("os", "other", "eq", "Ubuntu"),
        ("git", "", "eq", "branch"),
    ] {
        let mut m = configured(7);
        let spec = &mut m.labs[0].environment_checks[0];
        spec.kind = kind.into();
        spec.selector = selector.into();
        spec.operator = operator.into();
        spec.expected = expected.into();
        assert_eq!(
            validate_manifest(&m).unwrap_err(),
            "LOCAL_COURSE_ENVIRONMENT_INVALID"
        );
    }
}

#[test]
fn environment_duplicate_unknown_and_reflection_requirements_are_rejected() {
    let mut m = configured(7);
    let duplicate = m.labs[0].environment_checks[0].clone();
    m.labs[0].environment_checks.push(duplicate);
    assert_eq!(
        validate_manifest(&m).unwrap_err(),
        "LOCAL_COURSE_ENVIRONMENT_INVALID"
    );
    let mut m = configured(7);
    m.labs[0].activity_kind = "reflection".into();
    assert_eq!(
        validate_manifest(&m).unwrap_err(),
        "LOCAL_COURSE_ENVIRONMENT_INVALID"
    );
    let mut value = serde_json::to_value(configured(7)).unwrap();
    value["labs"][0]["environmentChecks"][0]["command"] = "touch unwanted".into();
    assert!(serde_json::from_value::<LocalCourseManifest>(value).is_err());
}

fn observation(spec: &environment::CheckSpec) -> serde_json::Value {
    serde_json::json!({"source":{"kind":"directory","commit":"","branch":"","dirty":null,"untracked":null,"status":"","message":""},
        "host":{"os":"Ubuntu","architecture":"x86_64"},
        "tools":[{"name":"cc","status":"OBSERVED","path":"/usr/bin/cc","version":"compiler 1","sha256":"a".repeat(64),"message":""}],
        "checks":[{"spec":spec,"observed":"compiler 1","status":"MATCH","message":""}]})
}

#[test]
fn environment_decoder_recomputes_comparisons_and_binds_exact_requirement() {
    let mut spec = configured(7).labs[0].environment_checks.remove(0);
    spec.operator = "eq".into();
    spec.expected = "compiler 2".into();
    let value = observation(&spec);
    let decoded =
        environment::decode(&serde_json::to_vec(&value).unwrap(), &[spec.clone()]).unwrap();
    assert_eq!(decoded.checks[0].status, "MISMATCH");
    let mut changed = value.clone();
    changed["checks"][0]["observed"] = "compiler 2".into();
    assert!(environment::decode(&serde_json::to_vec(&changed).unwrap(), &[spec.clone()]).is_err());
    let mut changed = value;
    changed["checks"][0]["spec"]["id"] = "other".into();
    assert!(environment::decode(&serde_json::to_vec(&changed).unwrap(), &[spec]).is_err());
}

#[test]
fn environment_decoder_redacts_before_ui_and_refuses_oversize_or_unknown_fields() {
    let spec = configured(7).labs[0].environment_checks.remove(0);
    let mut value = observation(&spec);
    value["tools"][0]["version"] = "password=secret-demo-value".into();
    value["checks"][0]["observed"] = "password=secret-demo-value".into();
    let decoded =
        environment::decode(&serde_json::to_vec(&value).unwrap(), &[spec.clone()]).unwrap();
    assert!(!serde_json::to_string(&decoded)
        .unwrap()
        .contains("secret-demo-value"));
    assert!(environment::decode(&vec![b' '; 65_537], &[]).is_err());
    value["executedAction"] = true.into();
    assert!(environment::decode(&serde_json::to_vec(&value).unwrap(), &[spec]).is_err());
}

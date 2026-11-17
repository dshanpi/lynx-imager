use super::*;

fn specs() -> Vec<environment::CheckSpec> {
    serde_json::from_value(serde_json::json!([
        {"id":"sdk","title":"SDK","kind":"file","selector":"shared-sdk/VERSION","operator":"sha256","expected":"a".repeat(64),"remediation":"Restore the declared SDK."},
        {"id":"compiler","title":"Compiler","kind":"tool","selector":"cc","operator":"present","expected":"","remediation":"Install the declared compiler."}
    ])).unwrap()
}
fn observation() -> serde_json::Value {
    let specs = specs();
    serde_json::json!({"source":{"kind":"directory","commit":"","branch":"","dirty":null,"untracked":null,"status":"","message":""},
        "host":{"os":"Ubuntu","architecture":"x86_64"},
        "tools":[{"name":"cc","status":"OBSERVED","path":"/usr/bin/compiler","version":"compiler 1","sha256":"b".repeat(64),"message":""}],
        "checks":[{"spec":specs[0],"observed":"a".repeat(64),"status":"MATCH","message":""},{"spec":specs[1],"observed":"compiler 1","status":"MATCH","message":""}]})
}
fn seal(value: serde_json::Value) -> Result<environment_receipt::Receipt, String> {
    environment_receipt::seal(&serde_json::to_vec(&value).unwrap(), &specs())
}

#[test]
fn environment_execution_receipt_rejects_missing_mismatched_and_unverified_requirements() {
    for (status, expected) in [
        ("MISSING", environment_receipt::CHANGED),
        ("UNVERIFIED", environment_receipt::UNAVAILABLE),
    ] {
        let mut value = observation();
        value["checks"][0]["status"] = status.into();
        value["checks"][0]["observed"] = "".into();
        assert_eq!(seal(value).unwrap_err(), expected);
    }
    let mut value = observation();
    value["checks"][0]["observed"] = "f".repeat(64).into();
    assert_eq!(
        seal(value).unwrap_err(),
        environment_receipt::CHANGED,
        "a forged MATCH is recomputed"
    );
}

#[test]
fn environment_execution_receipt_tracks_tool_content_path_and_version_but_not_unrelated_git_state()
{
    let original = Some(seal(observation()).unwrap());
    for field in ["sha256", "path", "version"] {
        let mut value = observation();
        value["tools"][0][field] = match field {
            "sha256" => "c".repeat(64),
            "path" => "/usr/bin/other-compiler".into(),
            _ => "compiler 2".into(),
        }
        .into();
        if field == "version" {
            value["checks"][1]["observed"] = "compiler 2".into();
        }
        let changed = Some(seal(value).unwrap());
        assert_eq!(
            environment_receipt::compare(&original, &changed, &specs()).unwrap_err(),
            environment_receipt::CHANGED
        );
    }
    let mut value = observation();
    value["source"]["branch"] = "new branch".into();
    value["source"]["status"] = "?? generated-output".into();
    assert_eq!(Some(seal(value).unwrap()), original);
}

#[test]
fn environment_execution_receipt_redacts_display_without_collapsing_raw_identity() {
    let mut value = observation();
    value["tools"][0]["version"] = "password=first-secret-value".into();
    value["checks"][1]["observed"] = "password=first-secret-value".into();
    let first = seal(value.clone()).unwrap();
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("first-secret-value"));
    value["tools"][0]["version"] = "password=second-secret-value".into();
    value["checks"][1]["observed"] = "password=second-secret-value".into();
    assert_ne!(first, seal(value).unwrap());
}

#[test]
fn environment_execution_receipt_is_bounded_and_bound_to_exact_signed_declarations() {
    let receipt = Some(seal(observation()).unwrap());
    environment_receipt::require(&receipt, &specs()).unwrap();
    assert_eq!(
        environment_receipt::require(&None, &specs()).unwrap_err(),
        environment_receipt::LEGACY
    );
    environment_receipt::require(&None, &[]).unwrap();
    let mut changed = specs();
    changed[0].selector = "another-sdk/VERSION".into();
    assert_eq!(
        environment_receipt::require(&receipt, &changed).unwrap_err(),
        environment_receipt::UNAVAILABLE
    );
    let original = serde_json::to_value(receipt.unwrap()).unwrap();
    for (field, value) in [
        ("version", serde_json::json!(99)),
        ("identitySha256", serde_json::json!("invalid")),
        ("observations", serde_json::json!([])),
    ] {
        let mut bad = original.clone();
        bad[field] = value;
        let r: environment_receipt::Receipt = serde_json::from_value(bad).unwrap();
        assert!(!r.valid());
    }
    let mut bad = original.clone();
    bad["observations"][0]["value"] = "x".repeat(129).into();
    assert!(!serde_json::from_value::<environment_receipt::Receipt>(bad)
        .unwrap()
        .valid());
    let mut bad = original;
    bad["executedOnBoard"] = true.into();
    assert!(serde_json::from_value::<environment_receipt::Receipt>(bad).is_err());
}

#[test]
fn environment_execution_report_checks_every_required_activity_and_caches_identical_observations() {
    let expected = Some(seal(observation()).unwrap());
    let first = environment_report::Plan {
        step_id: "early".into(),
        attempt: 1,
        digest: "first".into(),
        binding_sha256: Some("a".repeat(64)),
        specs: specs(),
        expected: expected.clone(),
    };
    let mut second = first.clone();
    second.step_id = "later".into();
    second.digest = "second".into();
    let mut calls = 0;
    let checks =
        environment_report::check_with(&[first.clone(), second.clone()], &"a".repeat(64), |_| {
            calls += 1;
            Ok(expected.clone())
        });
    assert_eq!(calls, 1);
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().all(|c| c.status == "MATCH"));
    assert_eq!(environment_report::aggregate("MATCH", &checks), "MATCH");
    let checks = environment_report::check_with(&[first.clone(), second], &"a".repeat(64), |_| {
        Err(environment_receipt::CHANGED.into())
    });
    assert!(checks.iter().all(|c| c.status == "CHANGED"));
    assert_eq!(environment_report::aggregate("MATCH", &checks), "CHANGED");
    let mut old = first;
    old.expected = None;
    let checks = environment_report::check_with(&[old], &"a".repeat(64), |_| {
        panic!("never backfill old execution")
    });
    assert_eq!(checks[0].status, "UNRECORDED");
    assert_eq!(
        environment_report::aggregate("MATCH", &checks),
        "UNRECORDED"
    );
}

#[test]
fn environment_execution_report_plan_keeps_early_required_dependency_and_ignores_optional() {
    let (db, session) = report_tests::database();
    let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
    manifest.schema_version = 7;
    manifest.labs[0].environment_checks = specs();
    let mut later = manifest.labs[0].clone();
    later.step_id = "later".into();
    later.number = 2;
    later.environment_checks.clear();
    let mut optional = manifest.labs[0].clone();
    optional.step_id = "optional".into();
    optional.number = 3;
    optional.required = false;
    manifest.labs.extend([later, optional]);
    db.execute(
        "UPDATE local_courses SET manifest_json=?1",
        [serde_json::to_string(&manifest).unwrap()],
    )
    .unwrap();
    let plans = environment_report::plan(&db, &session).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].step_id, "step-1");
    assert!(plans[0].expected.is_none());
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get(0))
        .unwrap();
    let checks = environment_report::check_with(&plans, &"a".repeat(64), |_| {
        panic!("old receipt must not be invented")
    });
    assert_eq!(checks[0].status, "UNRECORDED");
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM learning_evidence", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-1',2,'practicing',2,2)",[]).unwrap();
    assert!(environment_report::plan(&db, &session).is_err());
}

fn receipt_json() -> serde_json::Value {
    serde_json::json!({"version":1,"requirementsSha256":"d".repeat(64),"identitySha256":"e".repeat(64),"observations":[{"id":"sdk","kind":"file","value":"a".repeat(64),"toolSha256":"","pathSha256":"","toolVersion":""}]})
}

#[test]
fn environment_execution_snapshot_preserves_environment_identity() {
    let value = serde_json::json!({"version":2,"bindingSha256":"a".repeat(64),"rootSha256":"b".repeat(64),"treeSha256":"c".repeat(64),"entries":1,"bytes":0,"environment":receipt_json()});
    let snapshot: freshness::Snapshot = serde_json::from_value(value.clone()).unwrap();
    freshness::compare(&snapshot, &snapshot).unwrap();
    assert_eq!(serde_json::to_value(&snapshot).unwrap(), value);
    let mut changed = value;
    changed["environment"]["identitySha256"] = "f".repeat(64).into();
    let changed: freshness::Snapshot = serde_json::from_value(changed).unwrap();
    assert!(freshness::compare(&snapshot, &changed)
        .unwrap_err()
        .starts_with("LEARNING_INPUTS_CHANGED"));
}

#[test]
fn environment_execution_report_has_separate_environment_currency() {
    let (db, _) = report_tests::database();
    let report = serde_json::to_value(generate_local_report(&db, "r").unwrap()).unwrap();
    assert_eq!(
        report["currentWorkspace"]["environmentChecks"],
        serde_json::json!([])
    );
    assert_eq!(report["recordedSoftwareResult"], "PASS");
    assert_eq!(report["currentWorkspace"]["status"], "NOT_CHECKED");
}

#[test]
fn environment_execution_report_rejects_earlier_binding_even_when_observations_match() {
    let expected = Some(seal(observation()).unwrap());
    let plan = environment_report::Plan {
        step_id: "early".into(),
        attempt: 1,
        digest: "first".into(),
        binding_sha256: Some("a".repeat(64)),
        specs: specs(),
        expected: expected.clone(),
    };
    let checks = environment_report::check_with(&[plan], &"b".repeat(64), |_| {
        panic!("different execution binding must fail before observation")
    });
    assert_eq!(checks[0].status, "CHANGED");
}

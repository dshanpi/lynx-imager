use super::*;

pub(super) fn fixture() -> (Connection, manifest::VerifiedCourse, Selection) {
    let (course, mut s, mut p) = plan::tests::input();
    let c = p.execution_context.as_mut().unwrap();
    let mut canonical = serde_json::to_value(&*c).unwrap();
    canonical.as_object_mut().unwrap().remove("fingerprint");
    canonical.as_object_mut().unwrap().remove("observedAt");
    c.fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&canonical).unwrap()));
    s.run.result["executionContext"] = serde_json::to_value(&*c).unwrap();
    s.run.result["task"] = json!({"taskId":"build","commit":c.source_commit,"learningSourceFingerprint":c.source_fingerprint,"definitionSha256":c.definition_sha256,"environmentFacts":c.facts});
    s.run.result["logSha256"] = json!(p.items[1].raw_sha256);
    s.run.result["logBytes"] = json!(p.items[1].raw_bytes);
    s.scope = hex::encode(Sha256::digest(
        serde_json::to_vec(&json!({"user":7,"request":s.run.request,"binding":s.binding_digest}))
            .unwrap(),
    ));
    p.snapshot_sha256.clear();
    p.snapshot_sha256 = hex::encode(Sha256::digest(serde_json::to_vec(&p).unwrap()));
    let db = Connection::open_in_memory().unwrap();
    super::super::migrate(&db).unwrap();
    db.execute(
        "INSERT INTO learn_project_runs VALUES(?1,7,?2,?3,?4,?5,?6)",
        params![
            s.run.run_id,
            s.run.request.environment.enrollment_id,
            s.run.request.environment.manifest_sha256,
            s.run.request.activity_id,
            serde_json::to_string(&s).unwrap(),
            s.run.updated_at
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO learn_action_evidence_previews VALUES(7,?1,?2,?3)",
        params![p.run_id, s.scope, serde_json::to_string(&p).unwrap()],
    )
    .unwrap();
    let selection = Selection {
        batch_id: "batch_one".into(),
        environment: p.request.environment,
        unit_id: p.request.unit_id,
        runs: vec![RunRef {
            run_id: p.run_id,
            snapshot_sha256: p.snapshot_sha256,
        }],
    };
    (db, course, selection)
}
#[test]
fn selection_reads_real_sqlite_preview_and_freezes_session_with_claims() {
    let (mut db, course, selection) = fixture();
    let snapshot = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
    let first = snapshot.pending.unwrap();
    assert_eq!(first.route, Route::Session);
    assert_eq!(snapshot.frozen.evidence.len(), 3);
    assert_eq!(
        evidence_bytes(&db, 7, &snapshot.frozen, 0).unwrap(),
        b"ok\n"
    );
    assert_eq!(
        evidence_bytes(&db, 7, &snapshot.frozen, 1).unwrap(),
        b"diagnostic\n"
    );
    assert!(evidence_bytes(&db, 8, &snapshot.frozen, 0).is_err());
    assert_eq!(
        prepare(&mut db, 7, &selection, &course, &"b".repeat(64))
            .unwrap()
            .pending,
        Some(first)
    );
}
#[test]
fn mismatched_selection_or_changed_preview_never_claims_the_run() {
    for mutation in ["owner", "binding", "digest", "unit", "source", "preview"] {
        let (mut db, mut course, mut selection) = fixture();
        let mut owner = 7;
        let mut binding = "b".repeat(64);
        match mutation {
            "owner" => owner = 8,
            "binding" => binding = "c".repeat(64),
            "digest" => selection.runs[0].snapshot_sha256 = "c".repeat(64),
            "unit" => selection.unit_id = "other".into(),
            "source" => course.manifest.resources[1].commit = Some("d".repeat(40)),
            _ => {
                db.execute("UPDATE learn_action_evidence_previews SET record='{}'", [])
                    .unwrap();
            }
        }
        assert!(
            prepare(&mut db, owner, &selection, &course, &binding).is_err(),
            "accepted {mutation}"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learn_sync_run_claims", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

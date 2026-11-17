use super::*;
use serde_json::json;

fn frozen(id: &str, runs: &[&str]) -> FrozenBatch {
    FrozenBatch {
        batch_id: id.into(),
        enrollment_id: "enrollment_one".into(),
        release_id: "release_one".into(),
        manifest_sha256: "a".repeat(64),
        unit_id: "unit_one".into(),
        profile_id: "ubuntu".into(),
        binding_digest: "b".repeat(64),
        context_fingerprint: "c".repeat(64),
        instance_id: format!("instance_{id}"),
        runs: runs
            .iter()
            .map(|run| RunRef {
                run_id: (*run).into(),
                snapshot_sha256: "d".repeat(64),
            })
            .collect(),
        evidence: runs
            .iter()
            .map(|run| EvidenceRef {
                run_id: (*run).into(),
                kind: "stdout".into(),
                sha256: "e".repeat(64),
                size: 5,
                content_type: "text/plain".into(),
                observed_at: "2026-10-18T00:00:01Z".into(),
            })
            .collect(),
        event_body: json!({"baseRevision":1,"instanceId":format!("instance_{id}"),"events":[]})
            .to_string(),
    }
}
fn first(batch: &FrozenBatch) -> Request {
    Request::new(0, format!("{}_session", batch.batch_id), Route::Session,
        json!({"enrollmentId":batch.enrollment_id,"unitId":batch.unit_id,"instanceId":batch.instance_id}), None).unwrap()
}
fn session() -> Receipt {
    Receipt::Session {
        session_id: "session_one".into(),
        attempt_id: "attempt_one".into(),
        revision: 1,
    }
}
fn events() -> Request {
    Request::new(
        1,
        "events_key".into(),
        Route::Events {
            attempt_id: "attempt_one".into(),
        },
        json!({"baseRevision":1,"instanceId":"instance_batch_one","events":[]}),
        None,
    )
    .unwrap()
}
fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    db
}

#[test]
fn restart_keeps_exact_request_bytes_key_and_account_scope() {
    let file = std::env::temp_dir().join(format!(
        "lynx-sync-journal-{:032x}.sqlite",
        rand::random::<u128>()
    ));
    let mut db = Connection::open(&file).unwrap();
    migrate(&db).unwrap();
    let batch = frozen("batch_one", &["run_one", "run_two"]);
    let request = first(&batch);
    create(&mut db, 7, &batch, &request).unwrap();
    drop(db);
    let db = Connection::open(&file).unwrap();
    migrate(&db).unwrap();
    let restored = load(&db, 7, "batch_one").unwrap();
    assert_eq!(restored.pending, Some(request));
    assert_eq!(restored.frozen, batch);
    assert!(load(&db, 8, "batch_one").is_err());
    assert_eq!(restored.revision, 0);
    drop(db);
    std::fs::remove_file(file).unwrap();
}

#[test]
fn same_batch_replay_is_exact_and_runs_cannot_be_reassigned() {
    let mut db = db();
    let batch = frozen("batch_one", &["run_one"]);
    let request = first(&batch);
    create(&mut db, 7, &batch, &request).unwrap();
    create(&mut db, 7, &batch, &request).unwrap();
    let other = frozen("batch_two", &["run_one"]);
    assert!(create(&mut db, 7, &other, &first(&other)).is_err());
    assert!(create(&mut db, 8, &other, &first(&other)).is_err());
    let mut changed = batch.clone();
    changed.runs[0].snapshot_sha256 = "e".repeat(64);
    assert!(create(&mut db, 7, &changed, &first(&changed)).is_err());
    assert!(load(&db, 7, "batch_two").is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM learn_sync_run_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn failed_initial_enqueue_rolls_back_business_record_and_all_run_claims() {
    let mut db = db();
    db.execute_batch("CREATE TRIGGER fail_request BEFORE INSERT ON learn_sync_requests BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let batch = frozen("batch_one", &["run_one", "run_two"]);
    assert!(create(&mut db, 7, &batch, &first(&batch)).is_err());
    for table in [
        "learn_sync_batches",
        "learn_sync_run_claims",
        "learn_sync_requests",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    db.execute_batch("DROP TRIGGER fail_request").unwrap();
    create(&mut db, 7, &batch, &first(&batch)).unwrap();
}

#[test]
fn receipt_and_next_request_are_atomic_and_stale_ack_cannot_advance() {
    let mut db = db();
    let batch = frozen("batch_one", &["run_one"]);
    let original = first(&batch);
    create(&mut db, 7, &batch, &original).unwrap();
    db.execute_batch("CREATE TRIGGER fail_next BEFORE INSERT ON learn_sync_requests WHEN NEW.sequence=1 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(acknowledge(
        &mut db,
        7,
        "batch_one",
        &original,
        &session(),
        Some(&events())
    )
    .is_err());
    assert_eq!(
        load(&db, 7, "batch_one").unwrap().pending,
        Some(original.clone())
    );
    assert!(receipt(&db, 7, "batch_one", 0).unwrap().is_none());
    db.execute_batch("DROP TRIGGER fail_next").unwrap();
    acknowledge(
        &mut db,
        7,
        "batch_one",
        &original,
        &session(),
        Some(&events()),
    )
    .unwrap();
    assert_eq!(load(&db, 7, "batch_one").unwrap().pending, Some(events()));
    assert_eq!(receipt(&db, 7, "batch_one", 0).unwrap(), Some(session()));
    assert!(acknowledge(
        &mut db,
        7,
        "batch_one",
        &original,
        &session(),
        Some(&events())
    )
    .is_err());
    assert!(acknowledge(&mut db, 8, "batch_one", &events(), &session(), None).is_err());
}

#[test]
fn corruption_does_not_silently_recreate_requests_or_release_claims() {
    for sql in [
        "UPDATE learn_sync_batches SET frozen='{}'",
        "UPDATE learn_sync_requests SET request='{}'",
        "UPDATE learn_sync_batches SET revision=4",
        "DELETE FROM learn_sync_run_claims",
    ] {
        let mut db = db();
        let batch = frozen("batch_one", &["run_one"]);
        create(&mut db, 7, &batch, &first(&batch)).unwrap();
        db.execute_batch(sql).unwrap();
        assert!(load(&db, 7, "batch_one").is_err(), "accepted {sql}");
    }
}

#[test]
fn bounded_ids_routes_and_receipts_cannot_store_upload_credentials() {
    assert!(Request::new(
        0,
        "key".into(),
        Route::Events {
            attempt_id: "../account".into()
        },
        json!({}),
        None
    )
    .is_err());
    assert!(Request::new(
        0,
        "key".into(),
        Route::Session,
        json!({"blob":"x".repeat(MAX_REQUEST_BYTES)}),
        None
    )
    .is_err());
    assert!(serde_json::from_value::<Receipt>(json!({"kind":"grant","uploadId":"upload_one","expiresAt":"2026-10-18T10:00:00Z","url":"https://private/?signature=secret"})).is_err());
    let mut db = db();
    let batch = frozen("batch_one", &["run_one", "run_one"]);
    assert!(create(&mut db, 7, &batch, &first(&batch)).is_err());
    let batch = frozen("batch_one", &["run_one"]);
    let mut request = first(&batch);
    request.body = "changed bytes".into();
    assert!(create(&mut db, 7, &batch, &request).is_err());
}

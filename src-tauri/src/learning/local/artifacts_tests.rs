use super::*;
use serde_json::json;

fn manifest_value() -> serde_json::Value {
    let mut value = serde_json::to_value(super::tests::manifest_v2_with_hil()).unwrap();
    value["schemaVersion"] = json!(5);
    value["workspace"] = json!({"remoteSubdir":"lesson","packageSourceRoot":"workspace"});
    value["labs"].as_array_mut().unwrap().truncate(1);
    value["labs"][0]["artifactOutputs"] = json!([{
        "artifactId":"metrics","title":"实际指标","path":"build/metrics.json",
        "mimeType":"application/json","maxBytes":65536
    }]);
    value
}

#[test]
fn signed_artifact_declaration_requires_explicit_supported_schema() {
    let value = manifest_value();
    let manifest: LocalCourseManifest = serde_json::from_value(value).unwrap();
    assert!(validate_manifest(&manifest).is_ok());
}

#[test]
fn older_schema_cannot_silently_ignore_artifact_declaration() {
    let mut value = manifest_value();
    value["schemaVersion"] = json!(4);
    let manifest: LocalCourseManifest = serde_json::from_value(value).unwrap();
    assert!(validate_manifest(&manifest).is_err());
}

fn spec() -> artifacts::ArtifactSpec {
    serde_json::from_value(json!({"artifactId":"file","title":"产物","path":"build/file.bin","mimeType":"application/octet-stream","maxBytes":1048576})).unwrap()
}
fn database() -> (Connection, LocalLearningSession) {
    let db = Connection::open_in_memory().unwrap();
    migrate(&db).unwrap();
    let manifest: LocalCourseManifest = serde_json::from_value(manifest_value()).unwrap();
    db.execute("INSERT INTO local_courses VALUES('t527-course','1.0.0-rc.1','Fixture','board','signer','hash',?1,'/unused',1)", [serde_json::to_string(&manifest).unwrap()]).unwrap();
    db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('artifact','t527-course','1.0.0-rc.1',1,'step-1','practicing',1,1)", []).unwrap();
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('artifact','step-1',1,'practicing',1,1)", []).unwrap();
    let session = get_session(&db, "artifact").unwrap();
    (db, session)
}
fn save(
    db: &mut Connection,
    session: &LocalLearningSession,
    data: Vec<u8>,
) -> artifacts::ArtifactScope {
    let result = record_observed_action_result(
        db,
        session,
        1,
        0,
        0,
        b"ok",
        ActionObservation {
            has_hil: true,
            snapshot: None,
            artifacts: vec![artifacts::collected(spec(), data).unwrap()],
        },
    )
    .unwrap();
    artifacts::ArtifactScope {
        session_id: session.session_id.clone(),
        step_id: session.current_step.clone(),
        attempt: 1,
        digest: result.output_digest,
    }
}
fn chunk(scope: &artifacts::ArtifactScope, sha: &str, offset: u64) -> artifacts::ChunkRequest {
    artifacts::ChunkRequest {
        scope: scope.clone(),
        artifact_id: "file".into(),
        sha256: sha.into(),
        offset,
    }
}

#[test]
fn artifact_declaration_rejects_paths_types_duplicates_and_budgets() {
    for (field, bad) in [
        ("path", json!("../private")),
        ("path", json!("/tmp/private")),
        ("mimeType", json!("text/html")),
        ("artifactId", json!("../id")),
        ("title", json!("")),
        ("maxBytes", json!(0)),
        ("maxBytes", json!(33554433)),
    ] {
        let mut value = manifest_value();
        value["labs"][0]["artifactOutputs"][0][field] = bad;
        let m: LocalCourseManifest = serde_json::from_value(value).unwrap();
        assert!(
            validate_manifest(&m).unwrap_err().contains("ARTIFACT"),
            "{field}"
        );
    }
    for duplicate in [true, false] {
        let mut m: LocalCourseManifest = serde_json::from_value(manifest_value()).unwrap();
        let mut other = m.labs[0].artifact_outputs[0].clone();
        if duplicate {
            other.path = "build/other.json".into();
        } else {
            other.artifact_id = "other".into();
        }
        m.labs[0].artifact_outputs.push(other);
        assert!(validate_manifest(&m).is_err());
    }
    let mut m: LocalCourseManifest = serde_json::from_value(manifest_value()).unwrap();
    m.labs[0].artifact_outputs = (0..3)
        .map(|i| artifacts::ArtifactSpec {
            artifact_id: format!("file-{i}"),
            path: format!("build/{i}"),
            max_bytes: 32 * 1024 * 1024,
            ..spec()
        })
        .collect();
    assert!(validate_manifest(&m).is_err());
    m.labs[0].artifact_outputs = vec![spec()];
    m.labs[0].activity_kind = "reflection".into();
    assert!(artifacts::validate(&m, &m.labs[0]).is_err());
    let mut value = manifest_value();
    value["labs"][0]["artifactOutputs"][0]["shell"] = json!("must reject");
    assert!(serde_json::from_value::<LocalCourseManifest>(value).is_err());
}

#[test]
fn artifact_declaration_budget_preserves_readable_command_evidence() {
    let mut m: LocalCourseManifest = serde_json::from_value(manifest_value()).unwrap();
    m.labs[0].artifact_outputs = (0..8)
        .map(|i| artifacts::ArtifactSpec {
            artifact_id: format!("file-{i}"),
            path: format!("build/{i}/{}", "\"".repeat(2000)),
            max_bytes: 1024,
            ..spec()
        })
        .collect();
    assert!(validate_manifest(&m).unwrap_err().contains("ARTIFACT"));
}

#[test]
fn artifact_formats_are_passive_bounded_and_validated() {
    for (mime, data) in [
        ("application/json", b"{}".to_vec()),
        (
            "text/plain",
            "文字 <script>never execute</script>".as_bytes().to_vec(),
        ),
        ("application/octet-stream", vec![]),
    ] {
        let mut s = spec();
        s.mime_type = mime.into();
        assert!(artifacts::collected(s, data).is_ok());
    }
    for (mime, data) in [
        ("application/json", b"not json".to_vec()),
        ("text/plain", vec![255]),
        ("audio/wav", b"RIFFbad".to_vec()),
        ("audio/flac", vec![0; 42]),
        ("video/mp4", vec![0; 12]),
    ] {
        let mut s = spec();
        s.mime_type = mime.into();
        assert!(artifacts::collected(s, data).is_err());
    }
    let mut s = spec();
    s.max_bytes = 2;
    assert!(artifacts::collected(s, vec![1, 2, 3]).is_err());
}

#[test]
fn artifacts_roundtrip_multiple_chunks_and_empty_file_without_writes() {
    for data in [
        Vec::new(),
        (0..(artifacts::CHUNK * 2 + 19))
            .map(|n| (n % 251) as u8)
            .collect(),
    ] {
        let (mut db, session) = database();
        let scope = save(&mut db, &session, data.clone());
        let before = db.total_changes();
        let list = artifacts::list(&db, &scope).unwrap();
        let meta = &list.items[0];
        assert_eq!(meta.sha256, sha256(&data));
        let mut actual = Vec::new();
        let mut offset = 0;
        loop {
            let part = artifacts::read_chunk(&db, &chunk(&scope, &meta.sha256, offset)).unwrap();
            let bytes = STANDARD.decode(part.data_base64).unwrap();
            assert!(bytes.len() <= artifacts::CHUNK);
            actual.extend(bytes);
            offset = part.next_offset;
            if offset == meta.bytes {
                break;
            }
        }
        assert_eq!(actual, data);
        assert_eq!(db.total_changes(), before);
        assert!(artifacts::read_chunk(&db, &chunk(&scope, &meta.sha256, 1)).is_err());
        assert!(artifacts::read_chunk(&db, &chunk(&scope, &"0".repeat(64), 0)).is_err());
    }
}

#[test]
fn artifact_reads_reject_wrong_record_scope_and_do_not_fill_legacy() {
    let (mut db, session) = database();
    let scope = save(&mut db, &session, b"file".to_vec());
    for field in ["session", "step", "attempt", "digest"] {
        let mut wrong = scope.clone();
        match field {
            "session" => wrong.session_id = "other".into(),
            "step" => wrong.step_id = "other".into(),
            "attempt" => wrong.attempt = 2,
            _ => wrong.digest = "a".repeat(64),
        };
        assert!(artifacts::list(&db, &wrong).is_err());
    }
    let legacy = append_evidence(
        &db,
        &session,
        "command",
        "software",
        "PASS",
        "legacy",
        &json!({}),
    )
    .unwrap();
    let old = artifacts::ArtifactScope {
        digest: legacy,
        ..scope.clone()
    };
    assert!(artifacts::list(&db, &old).unwrap().items.is_empty());
    let other = append_evidence(
        &db,
        &session,
        "photo",
        "user-camera",
        "RECORDED",
        "photo",
        &json!({"photo":true}),
    )
    .unwrap();
    assert!(artifacts::list(
        &db,
        &artifacts::ArtifactScope {
            digest: other,
            ..scope
        }
    )
    .is_err());
}

#[test]
fn artifact_corrupt_body_index_metadata_or_missing_chunk_is_rejected() {
    for corruption in ["body", "hash", "index", "metadata", "missing"] {
        let (mut db, session) = database();
        let scope = save(&mut db, &session, b"file".to_vec());
        let sha = sha256(b"file");
        match corruption {
            "body" => db
                .execute("UPDATE learning_artifact_chunks SET body=x'66616b65'", [])
                .unwrap(),
            "hash" => db
                .execute(
                    "UPDATE learning_artifact_chunks SET sha256=?1",
                    [sha256(b"fake")],
                )
                .unwrap(),
            "index" => db
                .execute("UPDATE learning_artifact_chunks SET chunk_index=1", [])
                .unwrap(),
            "metadata" => db
                .execute("UPDATE learning_evidence SET detail_json='{}'", [])
                .unwrap(),
            _ => db
                .execute("DELETE FROM learning_artifact_chunks", [])
                .unwrap(),
        };
        assert!(
            artifacts::read_chunk(&db, &chunk(&scope, &sha, 0)).is_err(),
            "{corruption}"
        );
    }
}

#[test]
fn artifact_write_failure_rolls_back_command_and_all_chunks() {
    let (mut db, session) = database();
    db.execute_batch("CREATE TRIGGER reject_chunk BEFORE INSERT ON learning_artifact_chunks WHEN NEW.chunk_index=1 BEGIN SELECT RAISE(FAIL,'test'); END;").unwrap();
    let item = artifacts::collected(spec(), vec![1; artifacts::CHUNK + 1]).unwrap();
    assert!(record_observed_action_result(
        &mut db,
        &session,
        1,
        0,
        0,
        b"ok",
        ActionObservation {
            has_hil: false,
            snapshot: None,
            artifacts: vec![item]
        }
    )
    .is_err());
    for table in ["learning_evidence", "learning_artifact_chunks"] {
        assert_eq!(
            db.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(get_session(&db, "artifact").unwrap().state, "practicing");
}

#[test]
fn failed_command_cannot_publish_success_artifacts() {
    let (mut db, session) = database();
    let item = artifacts::collected(spec(), b"old file".to_vec()).unwrap();
    assert!(record_observed_action_result(
        &mut db,
        &session,
        1,
        1,
        0,
        b"failed",
        ActionObservation {
            has_hil: false,
            snapshot: None,
            artifacts: vec![item]
        }
    )
    .is_err());
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM learning_evidence", [], |r| r.get(0))
            .unwrap(),
        0
    );
}

#[test]
fn historical_artifact_survives_later_attempt_and_keeps_original_bytes() {
    let (mut db, session) = database();
    let first = save(&mut db, &session, b"first".to_vec());
    db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('artifact','step-1',2,'practicing',2,2)",[]).unwrap();
    let result = record_observed_action_result(
        &mut db,
        &session,
        2,
        0,
        0,
        b"new",
        ActionObservation {
            has_hil: true,
            snapshot: None,
            artifacts: vec![artifacts::collected(spec(), b"second".to_vec()).unwrap()],
        },
    )
    .unwrap();
    let second = artifacts::ArtifactScope {
        attempt: 2,
        digest: result.output_digest,
        ..first.clone()
    };
    for (scope, expected) in [
        (&first, b"first".as_slice()),
        (&second, b"second".as_slice()),
    ] {
        let bytes = artifacts::read_chunk(&db, &chunk(scope, &sha256(expected), 0)).unwrap();
        assert_eq!(STANDARD.decode(bytes.data_base64).unwrap(), expected);
    }
}

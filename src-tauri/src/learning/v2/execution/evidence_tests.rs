use super::*;

fn saved() -> Stored {
    let mut s = super::super::tests::saved_fixture();
    s.run.state = "succeeded".into();
    s.run.result = json!({"exitCode":0,"startedAt":"2026-10-18T00:00:00Z","finishedAt":"2026-10-18T00:00:01Z","logSha256":sha(b"diagnostic\n"),"logBytes":11});
    s
}
fn part(s: &Stored, kind: &str, bytes: &[u8], offset: usize) -> Value {
    let end = bytes.len().min(offset + CHUNK);
    json!({"runId":s.run.run_id,"kind":kind,"state":s.run.state,
        "startedAt":s.run.result["startedAt"],"finishedAt":s.run.result["finishedAt"],
        "contentType":if kind=="exit_status" {"application/json"} else {"text/plain"},
        "sha256":sha(bytes),"size":bytes.len(),"rawSha256":sha(bytes),"rawBytes":bytes.len(),
        "redacted":false,"outputTruncated":s.run.state=="output_limit","collectorVersion":"lynx-project-evidence/1",
        "offset":offset,"nextOffset":end,"dataBase64":STANDARD.encode(&bytes[offset..end])})
}
fn assembled(s: &Stored, kind: &str, bytes: &[u8]) -> EvidenceItem {
    let mut a = Assembler::new(kind);
    loop {
        a.push(s, part(s, kind, bytes, a.bytes.len())).unwrap();
        if a.done {
            break;
        }
    }
    a.finish(s).unwrap()
}
fn preview(s: &Stored) -> EvidencePreview {
    let mut p = EvidencePreview {
        snapshot_sha256: String::new(),
        run_id: s.run.run_id.clone(),
        request: s.run.request.clone(),
        binding_digest: s.binding_digest.clone(),
        collected_at: "2026-10-18T00:00:02Z".into(),
        trust: "client_observation".into(),
        sync_status: "local_only".into(),
        execution_context: None,
        items: vec![
            assembled(s, "stdout", b"complete stdout\n"),
            assembled(s, "build_log", b"diagnostic\n"),
            assembled(s, "exit_status", b"0"),
        ],
    };
    p.snapshot_sha256 = preview_digest(&p).unwrap();
    p
}

#[test]
fn complete_stdout_is_validated_without_stderr_or_tail_substitution() {
    let s = saved();
    assert_eq!(
        assembled(&s, "stdout", b"complete stdout\n").text,
        "complete stdout\n"
    );
    assert_eq!(assembled(&s, "stdout", b"").size, 0);
    let bytes = "学习完整输出\n".repeat(10000).into_bytes();
    let mut a = Assembler::new("stdout");
    a.push(&s, part(&s, "stdout", &bytes, 0)).unwrap();
    let request = a.request(&s);
    assert_eq!(request["offset"], CHUNK);
    assert_eq!(request["expectedSha256"], sha(&bytes));
    assert_eq!(request["expectedRawSha256"], sha(&bytes));
    assert_eq!(assembled(&s, "stdout", &bytes).text.as_bytes(), bytes);
}

#[test]
fn rejects_mismatched_scope_metadata_types_ranges_and_unknown_fields() {
    let s = saved();
    for (field, bad) in [
        ("runId", json!("other")),
        ("kind", json!("serial_log")),
        ("state", json!("running")),
        ("startedAt", json!("yesterday")),
        ("finishedAt", json!("2026-10-17T00:00:00Z")),
        ("contentType", json!("text/html")),
        ("collectorVersion", json!("other")),
        ("size", json!(LIMIT + 1)),
        ("rawBytes", json!(LIMIT)),
        ("sha256", json!("F".repeat(64))),
        ("redacted", json!(true)),
        ("outputTruncated", json!(true)),
        ("offset", json!(1)),
        ("nextOffset", json!(0)),
        ("dataBase64", json!("###")),
        ("arbitraryPath", json!("/etc/passwd")),
    ] {
        let mut value = part(&s, "stdout", b"hello", 0);
        value[field] = bad;
        let mut a = Assembler::new("stdout");
        assert!(a.push(&s, value).is_err(), "accepted {field}");
    }
    let mut value = part(&s, "stdout", b"hello", 0);
    value["size"] = json!("5");
    assert!(Assembler::new("stdout").push(&s, value).is_err());
}

#[test]
fn rejects_partial_duplicate_and_changed_chunks_including_hidden_raw_changes() {
    let s = saved();
    let bytes = vec![b'a'; CHUNK + 5];
    let mut a = Assembler::new("stdout");
    a.push(&s, part(&s, "stdout", &bytes, 0)).unwrap();
    assert!(a.finish(&s).is_err());
    for field in ["sha256", "rawSha256"] {
        let mut a = Assembler::new("stdout");
        let mut first = part(&s, "stdout", &bytes, 0);
        first["redacted"] = json!(true);
        first["rawSha256"] = json!("c".repeat(64));
        a.push(&s, first).unwrap();
        let mut second = part(&s, "stdout", &bytes, CHUNK);
        second["redacted"] = json!(true);
        second["rawSha256"] = json!("c".repeat(64));
        second[field] = json!("d".repeat(64));
        assert!(a.push(&s, second).is_err());
    }
    let mut a = Assembler::new("stdout");
    a.push(&s, part(&s, "stdout", b"hi", 0)).unwrap();
    assert!(a.push(&s, part(&s, "stdout", b"hi", 0)).is_err());
}

#[test]
fn recomputes_full_digest_and_requires_valid_utf8() {
    let s = saved();
    for bytes in [b"altered".as_slice(), &[255, 254]] {
        let mut v = part(&s, "stdout", bytes, 0);
        if bytes[0] != 255 {
            v["sha256"] = json!("a".repeat(64));
            v["rawSha256"] = json!("a".repeat(64));
        }
        let mut a = Assembler::new("stdout");
        a.push(&s, v).unwrap();
        assert!(a.finish(&s).is_err());
    }
}

#[test]
fn checks_exit_status_log_provenance_and_legacy_records() {
    let mut s = saved();
    for text in [
        b"1".as_slice(),
        b"null",
        b"true",
        b"0.0",
        b"{\"exitCode\":0}",
    ] {
        let mut a = Assembler::new("exit_status");
        a.push(&s, part(&s, "exit_status", text, 0)).unwrap();
        assert!(a.finish(&s).is_err());
    }
    assert!(Assembler::new("build_log")
        .push(&s, part(&s, "build_log", b"wrong log", 0))
        .is_err());
    for key in ["exitCode", "startedAt", "finishedAt"] {
        let mut legacy = saved();
        legacy.run.result.as_object_mut().unwrap().remove(key);
        assert!(Assembler::new("stdout")
            .push(&legacy, part(&s, "stdout", b"hello", 0))
            .is_err());
    }
    s.run.state = "cancelled".into();
    s.run.result["exitCode"] = json!(-15);
    assert_eq!(assembled(&s, "exit_status", b"-15").text, "-15");
}

#[test]
fn sqlite_snapshot_restores_without_remote_and_is_account_scoped() {
    let dir = TestDir::new();
    let path = dir.path().join("learn.db");
    let local = LocalLearningState::open(&path, dir.path().into()).unwrap();
    let mut s = saved();
    store(&local, 7, &mut s).unwrap();
    let p = preview(&s);
    persist(&local, 7, &s, &p).unwrap();
    persist(&local, 7, &s, &p).unwrap();
    drop(local);
    let reopened = LocalLearningState::open(&path, dir.path().into()).unwrap();
    let db = reopened.db.lock().unwrap();
    assert_eq!(
        restore(&db, 7, &s).unwrap().unwrap().snapshot_sha256,
        p.snapshot_sha256
    );
    assert!(restore(&db, 8, &s).unwrap().is_none());
    s.run.request.unit_id = "other".into();
    assert!(restore(&db, 7, &s).is_err());
}

#[test]
fn incomplete_stale_or_other_owner_collection_never_persists() {
    let local =
        LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir()).unwrap();
    let mut s = saved();
    store(&local, 7, &mut s).unwrap();
    let p = preview(&s);
    let mut incomplete = p.clone();
    incomplete.items.pop();
    incomplete.snapshot_sha256 = preview_digest(&incomplete).unwrap();
    assert!(persist(&local, 7, &s, &incomplete).is_err());
    assert!(persist(&local, 8, &s, &p).is_err());
    let stale = load(&local, 7, &s.run.run_id).unwrap();
    store(&local, 7, &mut s).unwrap();
    assert!(persist(&local, 7, &stale, &p).is_err());
    assert!(restore(&local.db.lock().unwrap(), 7, &s).unwrap().is_none());
}

#[test]
fn immutable_preview_rejects_replacement_corruption_and_authority_upgrade() {
    let local =
        LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir()).unwrap();
    let mut s = saved();
    store(&local, 7, &mut s).unwrap();
    let p = preview(&s);
    persist(&local, 7, &s, &p).unwrap();
    let mut changed = p.clone();
    changed.collected_at = "2026-10-18T00:00:03Z".into();
    changed.snapshot_sha256 = preview_digest(&changed).unwrap();
    assert!(persist(&local, 7, &s, &changed).is_err());
    changed = p.clone();
    changed.trust = "server_verified".into();
    changed.snapshot_sha256 = preview_digest(&changed).unwrap();
    assert!(validate_preview(&s, &changed).is_err());
    changed = p.clone();
    changed.items[0].text = "corrupted".into();
    local
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE learn_action_evidence_previews SET record=?1",
            params![serde_json::to_string(&changed).unwrap()],
        )
        .unwrap();
    assert!(restore(&local.db.lock().unwrap(), 7, &s).is_err());
}

#[test]
fn legacy_snapshot_wire_and_digest_do_not_change_when_context_is_absent() {
    let s = saved();
    let p = preview(&s);
    let raw = serde_json::to_value(&p).unwrap();
    assert!(raw.get("executionContext").is_none());
    let restored: EvidencePreview = serde_json::from_value(raw).unwrap();
    assert!(restored.execution_context.is_none());
    assert_eq!(preview_digest(&restored).unwrap(), p.snapshot_sha256);
    validate_preview(&s, &restored).unwrap();
}

#[test]
fn recorded_context_is_part_of_immutable_preview_and_cannot_be_replaced() {
    let local =
        LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir()).unwrap();
    let mut s = saved();
    let mut c = json!({"collectorVersion":"lynx-project-context/1","sourceCommit":"a".repeat(40),"sourceFingerprint":"b".repeat(64),"definitionSha256":"c".repeat(64),"facts":{"os":"Ubuntu 测试","architecture":"aarch64","pythonVersion":"3.12.0"}});
    c["fingerprint"] = json!(sha(&serde_json::to_vec(&c).unwrap()));
    c["observedAt"] = json!("2026-10-18T00:00:00Z");
    s.run.result["task"] = json!({"commit":c["sourceCommit"],"learningSourceFingerprint":c["sourceFingerprint"],"definitionSha256":c["definitionSha256"],"environmentFacts":c["facts"]});
    s.run.result["executionContext"] = c;
    store(&local, 7, &mut s).unwrap();
    let mut p = preview(&s);
    p.execution_context = super::super::provenance::context(&s).unwrap();
    p.snapshot_sha256 = preview_digest(&p).unwrap();
    persist(&local, 7, &s, &p).unwrap();
    assert_eq!(
        restore(&local.db.lock().unwrap(), 7, &s)
            .unwrap()
            .unwrap()
            .execution_context,
        p.execution_context
    );
    p.execution_context = None;
    p.snapshot_sha256 = preview_digest(&p).unwrap();
    assert!(validate_preview(&s, &p).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn production_python_chunks_round_trip_through_rust_validation() {
    // Real packaged Python protocol, not a second implementation or production host.
    let s = saved();
    let dir = TestDir::new();
    let job = dir.path().join("test_run");
    std::fs::create_dir(&job).unwrap();
    use std::os::unix::fs::PermissionsExt;
    for path in [dir.path(), job.as_path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(job.join("status.json"),serde_json::to_vec(&json!({"state":"succeeded","exitCode":0,"startedAt":s.run.result["startedAt"],"finishedAt":s.run.result["finishedAt"]})).unwrap()).unwrap();
    std::fs::write(
        job.join("prepared.json"),
        serde_json::to_vec(&json!({"request":{"scope":s.scope}})).unwrap(),
    )
    .unwrap();
    std::fs::write(job.join("stdout.log"), "中文\n".repeat(25000)).unwrap();
    std::fs::write(job.join("output.log"), b"diagnostic\n").unwrap();
    let script="import importlib.util,json,sys; spec=importlib.util.spec_from_file_location('runner',sys.argv[1]); m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);print(json.dumps(m.dispatch(json.load(sys.stdin),sys.argv[2])))";
    for kind in KINDS {
        let mut a = Assembler::new(kind);
        while !a.done {
            let mut child = std::process::Command::new("python3")
                .args([
                    "-c",
                    script,
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/src/learning/v2/project_runner.py"
                    ),
                    dir.path().to_str().unwrap(),
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(serde_json::to_string(&a.request(&s)).unwrap().as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            a.push(&s, serde_json::from_slice(&output.stdout).unwrap())
                .unwrap();
        }
        assert!(!a.finish(&s).unwrap().text.is_empty());
    }
}

struct TestDir(std::path::PathBuf);
impl TestDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lynx-evidence-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

//! New reviews wait for evidence; permitted unknown reviews retain exact replay.
use super::super::super::review;
use super::*;
use rusqlite::OptionalExtension;

// No SQLite guard is held while waiting for this gate or performing network IO.
pub(crate) static WRITE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GateRequest {
    #[serde(flatten)]
    pub pin: review::Pin,
    pub unit_id: String,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGate {
    pub pending: bool,
    pub attempt_id: Option<String>,
}

pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_review_permits(user_id INTEGER NOT NULL,request_id TEXT NOT NULL,body_sha256 TEXT NOT NULL,PRIMARY KEY(user_id,request_id)); CREATE TABLE IF NOT EXISTS learn_reviewed_sync_attempts(user_id INTEGER NOT NULL,enrollment_id TEXT NOT NULL,release_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,attempt_id TEXT NOT NULL,PRIMARY KEY(user_id,enrollment_id,release_id,manifest_sha256,attempt_id));")
}
pub(crate) fn gate(
    db: &Connection,
    owner: i64,
    pin: &review::Pin,
    unit: &str,
) -> Result<ReviewGate, String> {
    let mut statement=db.prepare("SELECT batch_id FROM learn_sync_batches WHERE user_id=?1 AND json_extract(frozen,'$.enrollmentId')=?2 AND json_extract(frozen,'$.releaseId')=?3 AND json_extract(frozen,'$.manifestSha256')=?4 AND json_extract(frozen,'$.unitId')=?5 ORDER BY rowid DESC").map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    let ids = statement
        .query_map(
            params![
                owner,
                pin.enrollment_id,
                pin.release_id,
                pin.manifest_sha256,
                unit
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut result = ReviewGate {
        pending: false,
        attempt_id: None,
    };
    for (index, id) in ids.iter().enumerate() {
        let snapshot = journal::load(db, owner, id)?;
        let attempt = journal::history(db, owner, id)?
            .into_iter()
            .find_map(|(_, receipt)| match receipt {
                Some(journal::Receipt::Session { attempt_id, .. }) => Some(attempt_id),
                _ => None,
            });
        if snapshot.pending.is_none() && attempt.is_none() {
            return Err("LEARN_SYNC_JOURNAL_INVALID".into());
        }
        result.pending |= snapshot.pending.is_some();
        if index == 0 {
            let reviewed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM learn_reviewed_sync_attempts WHERE user_id=?1 AND enrollment_id=?2 AND release_id=?3 AND manifest_sha256=?4 AND attempt_id=?5)",params![owner,pin.enrollment_id,pin.release_id,pin.manifest_sha256,attempt],|r|r.get(0)).map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
            if !reviewed {
                result.attempt_id = attempt;
            }
        }
    }
    Ok(result)
}
// Called only with validated server receipts, never a front-end claim.
pub(crate) fn reviewed(
    db: &Connection,
    owner: i64,
    pin: &review::Pin,
    attempt: &str,
) -> Result<(), String> {
    if !manifest::valid_id(attempt) {
        return Err("LEARN_SYNC_RECEIPT_INVALID".into());
    }
    db.execute(
        "INSERT OR IGNORE INTO learn_reviewed_sync_attempts VALUES(?1,?2,?3,?4,?5)",
        params![
            owner,
            pin.enrollment_id,
            pin.release_id,
            pin.manifest_sha256,
            attempt
        ],
    )
    .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    Ok(())
}
pub(crate) async fn owner(auth: &AuthState) -> Result<i64, super::super::super::LearnError> {
    user(auth)
        .await
        .map_err(|_| fault("LEARN_AUTH_REQUIRED".into()))
}
pub(crate) fn fault(code: String) -> super::super::super::LearnError {
    let (code, message) = match code.as_str() {
        "LEARN_AUTH_REQUIRED" => ("LEARN_AUTH_REQUIRED", "请先登录"),
        "LEARN_ACCOUNT_CHANGED" => ("LEARN_ACCOUNT_CHANGED", "登录账号已改变，请重新同步"),
        "LEARN_SYNC_PENDING" => ("LEARN_SYNC_PENDING", "本课时证据仍在同步，请先确认原批次"),
        "LEARN_SYNC_ATTEMPT_MISMATCH" => (
            "LEARN_SYNC_ATTEMPT_MISMATCH",
            "实践尝试已更新，请先同步评测结果",
        ),
        "LEARN_IDEMPOTENCY_CONFLICT" => (
            "LEARN_IDEMPOTENCY_CONFLICT",
            "原请求内容不一致，请保留恢复记录",
        ),
        "LEARN_EVIDENCE_STALE" => (
            "LEARN_EVIDENCE_STALE",
            "当前工程或环境已变化，请重新执行并同步实践证据",
        ),
        "LEARN_EVIDENCE_CHECK_UNAVAILABLE" => (
            "LEARN_EVIDENCE_CHECK_UNAVAILABLE",
            "无法核验当前工程，请恢复连接后重新检查",
        ),
        _ => (
            "LEARN_STORE_UNAVAILABLE",
            "本机恢复记录暂不可用，请保留原输入",
        ),
    };
    super::super::super::error(code, message, false)
}

fn permit_digest(request: &review::ReviewRequest, body: &Value) -> String {
    hex::encode(Sha256::digest(
        json!({"enrollmentId":request.pin.enrollment_id,"body":body})
            .to_string()
            .as_bytes(),
    ))
}
/// Exact previously permitted requests may already have committed remotely.
/// None means no current-practice probe is needed; it never grants a new permit.
pub(crate) fn review_plan(
    db: &Connection,
    owner: i64,
    request: &review::ReviewRequest,
    body: &Value,
) -> Result<Option<String>, String> {
    let previous: Option<String> = db
        .query_row(
            "SELECT body_sha256 FROM learn_review_permits WHERE user_id=?1 AND request_id=?2",
            params![owner, request.request_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    if let Some(previous) = previous {
        return if previous == permit_digest(request, body) {
            Ok(None)
        } else {
            Err("LEARN_IDEMPOTENCY_CONFLICT".into())
        };
    }
    let current = gate(db, owner, &request.pin, &request.unit_id)?;
    if current.pending {
        return Err("LEARN_SYNC_PENDING".into());
    }
    if current
        .attempt_id
        .as_ref()
        .is_some_and(|id| id != &request.attempt_id)
    {
        return Err("LEARN_SYNC_ATTEMPT_MISMATCH".into());
    }
    if request.attempt_id.is_empty() {
        return Ok(None);
    }
    let mut statement=db.prepare("SELECT batch_id FROM learn_sync_batches WHERE user_id=?1 AND json_extract(frozen,'$.enrollmentId')=?2 AND json_extract(frozen,'$.releaseId')=?3 AND json_extract(frozen,'$.manifestSha256')=?4 AND json_extract(frozen,'$.unitId')=?5").map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    let ids = statement
        .query_map(
            params![
                owner,
                request.pin.enrollment_id,
                request.pin.release_id,
                request.pin.manifest_sha256,
                request.unit_id
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut found = None;
    for id in ids {
        if journal::history(db,owner,&id)?.iter().any(|(_,r)|matches!(r,Some(journal::Receipt::Session{attempt_id,..}) if attempt_id==&request.attempt_id)) {
            if found.is_some(){return Err("LEARN_SYNC_JOURNAL_INVALID".into());}found=Some(id);
        }
    }
    Ok(found)
}

pub(crate) fn authorize(
    db: &mut Connection,
    owner: i64,
    request: &review::ReviewRequest,
    body: &Value,
) -> Result<(), String> {
    if owner <= 0 || !manifest::valid_id(&request.request_id) {
        return Err("LEARN_REVIEW_INVALID".into());
    }
    let digest = permit_digest(request, body);
    let tx = db.transaction().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let previous: Option<String> = tx
        .query_row(
            "SELECT body_sha256 FROM learn_review_permits WHERE user_id=?1 AND request_id=?2",
            params![owner, request.request_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    if let Some(previous) = previous {
        return if previous == digest {
            Ok(())
        } else {
            Err("LEARN_IDEMPOTENCY_CONFLICT".into())
        };
    }
    let current = gate(&tx, owner, &request.pin, &request.unit_id)?;
    if current.pending {
        return Err("LEARN_SYNC_PENDING".into());
    }
    if current
        .attempt_id
        .as_ref()
        .is_some_and(|id| id != &request.attempt_id)
    {
        return Err("LEARN_SYNC_ATTEMPT_MISMATCH".into());
    }
    tx.execute(
        "INSERT INTO learn_review_permits VALUES(?1,?2,?3)",
        params![owner, request.request_id, digest],
    )
    .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    tx.commit().map_err(|_| "LEARN_STORE_UNAVAILABLE".into())
}

#[tauri::command]
pub async fn learn_get_practice_review_gate(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    request: GateRequest,
) -> Result<ReviewGate, String> {
    let owner = user(&auth).await?;
    let result = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        gate(&db, owner, &request.pin, &request.unit_id)?
    };
    if user(&auth).await? != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn review_request() -> review::ReviewRequest {
        let (_, _, selection) = super::super::tests::fixture();
        serde_json::from_value(json!({"enrollmentId":selection.environment.enrollment_id,"releaseId":selection.environment.release_id,"manifestSha256":selection.environment.manifest_sha256,"unitId":selection.unit_id,"requestId":"review_one","expectedRevision":1,"attemptId":"","answers":{},"reflection":"original"})).unwrap()
    }
    fn body(r: &review::ReviewRequest) -> Value {
        json!({"enrollmentId":r.pin.enrollment_id,"releaseId":r.pin.release_id,"manifestSha256":r.pin.manifest_sha256,"unitId":r.unit_id,"expectedRevision":r.expected_revision,"attemptId":r.attempt_id,"answers":r.answers,"reflection":r.reflection})
    }
    #[test]
    fn new_review_waits_for_current_unit_batch_and_other_owner_is_isolated() {
        let (mut db, course, selection) = super::super::tests::fixture();
        migrate(&db).unwrap();
        let request = review_request();
        prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        assert!(
            gate(&db, 7, &request.pin, &request.unit_id)
                .unwrap()
                .pending
        );
        assert!(
            !gate(&db, 8, &request.pin, &request.unit_id)
                .unwrap()
                .pending
        );
        assert_eq!(
            authorize(&mut db, 7, &request, &body(&request)).unwrap_err(),
            "LEARN_SYNC_PENDING"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learn_review_permits", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let mut other = review_request();
        other.unit_id = "other_unit".into();
        authorize(&mut db, 7, &other, &body(&other)).unwrap();
    }
    #[test]
    fn unknown_permitted_review_replays_but_key_cannot_change_owner_or_body() {
        let (mut db, course, selection) = super::super::tests::fixture();
        migrate(&db).unwrap();
        let original = review_request();
        let original_body = body(&original);
        authorize(&mut db, 7, &original, &original_body).unwrap();
        prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        assert!(
            gate(&db, 7, &original.pin, &original.unit_id)
                .unwrap()
                .pending
        );
        // Approval was durably saved before an earlier POST whose result was lost.
        authorize(&mut db, 7, &original, &original_body).unwrap();
        let mut changed = review_request();
        changed.reflection = "changed".into();
        assert_eq!(
            authorize(&mut db, 7, &changed, &body(&changed)).unwrap_err(),
            "LEARN_IDEMPOTENCY_CONFLICT"
        );
        let mut fresh = review_request();
        fresh.request_id = "review_two".into();
        assert_eq!(
            authorize(&mut db, 7, &fresh, &body(&fresh)).unwrap_err(),
            "LEARN_SYNC_PENDING"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM learn_review_permits WHERE user_id=8",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn permit_survives_database_reopen_and_failed_insert_prevents_dispatch() {
        let path = std::env::temp_dir().join(format!(
            "lynx-review-permit-{:032x}.sqlite",
            rand::random::<u128>()
        ));
        let mut db = Connection::open(&path).unwrap();
        super::super::super::migrate(&db).unwrap();
        migrate(&db).unwrap();
        let original = review_request();
        authorize(&mut db, 7, &original, &body(&original)).unwrap();
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        migrate(&db).unwrap();
        authorize(&mut db, 7, &original, &body(&original)).unwrap();
        db.execute_batch("CREATE TRIGGER fail_permit BEFORE INSERT ON learn_review_permits BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        let mut next = review_request();
        next.request_id = "review_next".into();
        assert!(authorize(&mut db, 7, &next, &body(&next)).is_err());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn review_and_practice_writes_share_one_gate_without_holding_sqlite() {
        let (db, course, selection) = super::super::tests::fixture();
        let shared = Arc::new(std::sync::Mutex::new(db));
        let guard = WRITE_GATE.lock().await;
        let copy = shared.clone();
        let waiting = tokio::spawn(async move {
            let _write = WRITE_GATE.lock().await;
            let request = review_request();
            authorize(&mut copy.lock().unwrap(), 7, &request, &body(&request))
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        prepare(
            &mut shared.lock().unwrap(),
            7,
            &selection,
            &course,
            &"b".repeat(64),
        )
        .unwrap();
        drop(guard);
        assert_eq!(waiting.await.unwrap().unwrap_err(), "LEARN_SYNC_PENDING");
    }
    #[test]
    fn latest_completed_batch_does_not_hide_an_older_pending_profile() {
        let (mut db, course, selection) = super::super::tests::fixture();
        let first = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        let mut later = first.frozen.clone();
        later.batch_id = "batch_later".into();
        later.profile_id = "other_profile".into();
        later.instance_id = "instance_later".into();
        later.event_body = later
            .event_body
            .replace(&first.frozen.instance_id, &later.instance_id);
        for run in &mut later.runs {
            let old = run.run_id.clone();
            run.run_id = format!("later_{old}");
            later.event_body = later.event_body.replace(&old, &run.run_id);
            for evidence in &mut later.evidence {
                if evidence.run_id == old {
                    evidence.run_id = run.run_id.clone();
                }
            }
        }
        let request=Request::new(0,"later_session".into(),Route::Session,json!({"enrollmentId":later.enrollment_id,"unitId":later.unit_id,"instanceId":later.instance_id}),None).unwrap();
        journal::create(&mut db, 7, &later, &request).unwrap();
        let mut snapshot = journal::load(&db, 7, "batch_later").unwrap();
        while let Some(request) = snapshot.pending.clone() {
            let receipt = stages::tests::response(&request, &snapshot.frozen);
            snapshot = stages::accept(&mut db, 7, "batch_later", &request, &receipt).unwrap();
        }
        let request = review_request();
        reviewed(&db, 7, &request.pin, "attempt_one").unwrap();
        assert!(
            gate(&db, 7, &request.pin, &request.unit_id)
                .unwrap()
                .pending
        );
        assert_eq!(
            authorize(&mut db, 7, &request, &body(&request)).unwrap_err(),
            "LEARN_SYNC_PENDING"
        );
    }
    #[test]
    fn accepted_batch_requires_matching_attempt_until_server_review_is_observed() {
        let (mut db, course, selection) = super::super::tests::fixture();
        let mut snapshot = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        while let Some(request) = snapshot.pending.clone() {
            let ack = stages::tests::response(&request, &snapshot.frozen);
            snapshot = stages::accept(&mut db, 7, "batch_one", &request, &ack).unwrap();
        }
        let mut request = review_request();
        assert!(
            !gate(&db, 7, &request.pin, &request.unit_id)
                .unwrap()
                .pending
        );
        assert_eq!(
            authorize(&mut db, 7, &request, &body(&request)).unwrap_err(),
            "LEARN_SYNC_ATTEMPT_MISMATCH"
        );
        request.attempt_id = "attempt_one".into();
        assert_eq!(
            review_plan(&db, 7, &request, &body(&request))
                .unwrap()
                .as_deref(),
            Some("batch_one")
        );
        authorize(&mut db, 7, &request, &body(&request)).unwrap();
        assert!(review_plan(&db, 7, &request, &body(&request))
            .unwrap()
            .is_none());
        // Another owner or release cannot retire this batch's review requirement.
        reviewed(&db, 8, &request.pin, "attempt_one").unwrap();
        let mut other = request.pin.clone();
        other.release_id = "other".into();
        reviewed(&db, 7, &other, "attempt_one").unwrap();
        assert_eq!(
            gate(&db, 7, &request.pin, &request.unit_id)
                .unwrap()
                .attempt_id
                .as_deref(),
            Some("attempt_one")
        );
        reviewed(&db, 7, &request.pin, "attempt_one").unwrap();
        assert!(gate(&db, 7, &request.pin, &request.unit_id)
            .unwrap()
            .attempt_id
            .is_none());
        let mut next = review_request();
        next.request_id = "next_review".into();
        authorize(&mut db, 7, &next, &body(&next)).unwrap();
    }
    #[test]
    fn permit_digest_includes_enrollment_path_and_read_failure_is_closed() {
        let (mut db, _, _) = super::super::tests::fixture();
        let original = review_request();
        let payload = body(&original);
        authorize(&mut db, 7, &original, &payload).unwrap();
        let mut other = review_request();
        other.pin.enrollment_id = "other".into();
        assert_eq!(
            authorize(&mut db, 7, &other, &payload).unwrap_err(),
            "LEARN_IDEMPOTENCY_CONFLICT"
        );
        db.execute_batch("DROP TABLE learn_sync_batches").unwrap();
        let mut fresh = review_request();
        fresh.request_id = "different".into();
        assert!(authorize(&mut db, 7, &fresh, &body(&fresh)).is_err());
        // An already authorized exact replay still survives unrelated gate failure.
        assert!(review_plan(&db, 7, &original, &payload).unwrap().is_none());
        authorize(&mut db, 7, &original, &payload).unwrap();
    }
}

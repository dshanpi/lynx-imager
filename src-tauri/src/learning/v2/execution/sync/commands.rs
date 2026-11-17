//! Explicit, one-stage synchronization commands. Reloading a view performs no writes.
use super::*;
use journal::{Receipt, Snapshot};

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitScope {
    #[serde(flatten)]
    pub environment: environment::EnvironmentRequest,
    pub unit_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchView {
    pub batch_id: String,
    #[serde(flatten)]
    pub scope: UnitScope,
    pub run_ids: Vec<String>,
    pub evidence_count: usize,
    pub verified_count: usize,
    pub bytes: usize,
    pub status: String,
    pub stage: String,
    pub revision: u32,
    pub attempt_id: Option<String>,
    pub error_code: Option<String>,
    pub retryable: bool,
}
fn scope(batch: &FrozenBatch) -> UnitScope {
    UnitScope {
        environment: environment::EnvironmentRequest {
            enrollment_id: batch.enrollment_id.clone(),
            release_id: batch.release_id.clone(),
            manifest_sha256: batch.manifest_sha256.clone(),
            profile_id: batch.profile_id.clone(),
        },
        unit_id: batch.unit_id.clone(),
    }
}
fn view(db: &Connection, owner: i64, snapshot: Snapshot) -> Result<BatchView, String> {
    let history = journal::history(db, owner, &snapshot.frozen.batch_id)?;
    let error = journal::last_error(db, owner, &snapshot.frozen.batch_id, snapshot.revision)?;
    let pending = snapshot.pending.as_ref();
    let stage = match pending.map(|r| &r.route) {
        Some(Route::Session) => "session",
        Some(Route::Events { .. }) => "events",
        Some(Route::Evidence { .. }) => "evidence",
        Some(Route::Grant { .. }) => "grant",
        Some(Route::Upload { .. }) => "upload",
        Some(Route::Completion { .. }) => "completion",
        None => "done",
    };
    let status = if pending.is_none() {
        "accepted"
    } else if error.as_ref().is_some_and(|(_, retryable)| !*retryable) {
        "conflict"
    } else {
        "pending"
    };
    let attempt_id = history.iter().find_map(|(_, r)| match r {
        Some(Receipt::Session { attempt_id, .. }) => Some(attempt_id.clone()),
        _ => None,
    });
    let verified_count = history
        .iter()
        .filter(|(_, r)| matches!(r, Some(Receipt::Completion { .. })))
        .count();
    Ok(BatchView {
        scope: scope(&snapshot.frozen),
        batch_id: snapshot.frozen.batch_id,
        run_ids: snapshot
            .frozen
            .runs
            .iter()
            .map(|r| r.run_id.clone())
            .collect(),
        evidence_count: snapshot.frozen.evidence.len(),
        verified_count,
        bytes: snapshot.frozen.evidence.iter().map(|e| e.size).sum(),
        status: status.into(),
        stage: stage.into(),
        revision: snapshot.revision,
        attempt_id,
        error_code: error.as_ref().map(|(s, _)| s.clone()),
        retryable: error.is_none_or(|(_, r)| r),
    })
}
async fn check_owner(auth: &AuthState, owner: i64) -> Result<(), String> {
    if user(auth).await? != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(())
}
async fn permitted(
    auth: &AuthState,
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    owner: i64,
    batch: &FrozenBatch,
) -> Result<(), String> {
    let request = scope(batch).environment;
    let (checked, course) = environment::verified(auth, &request).await?;
    if checked != owner || !course.execution_enabled {
        return Err("LEARN_SYNC_NOT_PERMITTED".into());
    }
    let (binding, _, _) = environment::bound(local, workbench, owner, &request)?;
    if binding.config_digest != batch.binding_digest {
        return Err("LEARN_BINDING_CHANGED".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn learn_prepare_practice_sync(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: Selection,
) -> Result<BatchView, String> {
    let owner_before = user(&auth).await?;
    let _write = review_gate::WRITE_GATE.lock().await;
    check_owner(&auth, owner_before).await?;
    let (owner, course) = environment::verified(&auth, &request.environment).await?;
    if owner != owner_before {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    let (binding, _, _) = environment::bound(&local, &workbench, owner, &request.environment)?;
    check_owner(&auth, owner).await?;
    let result = {
        let mut db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        let saved = prepare(&mut db, owner, &request, &course, &binding.config_digest)?;
        view(&db, owner, saved)?
    };
    check_owner(&auth, owner).await?;
    Ok(result)
}

#[tauri::command]
pub async fn learn_list_practice_sync(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    request: UnitScope,
) -> Result<Vec<BatchView>, String> {
    let owner = user(&auth).await?;
    let result = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        list(&db, owner, &request)?
    };
    check_owner(&auth, owner).await?;
    Ok(result)
}
fn list(db: &Connection, owner: i64, request: &UnitScope) -> Result<Vec<BatchView>, String> {
    let e = &request.environment;
    let mut statement=db.prepare("SELECT batch_id FROM learn_sync_batches WHERE user_id=?1 AND json_extract(frozen,'$.enrollmentId')=?2 AND json_extract(frozen,'$.releaseId')=?3 AND json_extract(frozen,'$.manifestSha256')=?4 AND json_extract(frozen,'$.profileId')=?5 AND json_extract(frozen,'$.unitId')=?6 ORDER BY rowid DESC LIMIT 50").map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    let ids = statement
        .query_map(
            params![
                owner,
                e.enrollment_id,
                e.release_id,
                e.manifest_sha256,
                e.profile_id,
                request.unit_id
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    ids.iter()
        .map(|id| view(db, owner, journal::load(db, owner, id)?))
        .collect()
}

struct StageInput<'a> {
    request: &'a Request,
    batch: &'a FrozenBatch,
    history: &'a [(Request, Option<Receipt>)],
    bytes: Option<Vec<u8>>,
}
async fn send_stage(
    auth: &AuthState,
    owner: i64,
    base: &str,
    token: &str,
    input: StageInput<'_>,
) -> Result<Receipt, api::Fault> {
    let StageInput {
        request,
        batch,
        history,
        bytes,
    } = input;
    let changed = || api::Fault {
        code: "LEARN_ACCOUNT_CHANGED",
        retryable: false,
    };
    check_owner(auth, owner).await.map_err(|_| changed())?;
    let Route::Upload {
        upload_id, sha256, ..
    } = &request.route
    else {
        return Ok(api::post(base, token, request).await?.receipt);
    };
    let (grant_request, grant_receipt) = history
        .iter()
        .rev()
        .find_map(|(r, ack)| match ack {
            Some(Receipt::Grant { upload_id: id, .. }) if id == upload_id => {
                Some((r, ack.as_ref().unwrap()))
            }
            _ => None,
        })
        .ok_or_else(api::unknown)?;
    let Receipt::Grant { expires_at, .. } = grant_receipt else {
        return Err(api::unknown());
    };
    if chrono::DateTime::parse_from_rfc3339(expires_at).map_err(|_| api::unknown())?
        <= chrono::Utc::now()
    {
        return Ok(Receipt::GrantExpired {
            upload_id: upload_id.clone(),
        });
    }
    // Replay the exact grant POST to recover its URL, which is never stored locally.
    let delivery = api::post(base, token, grant_request).await?;
    if &delivery.receipt != grant_receipt {
        return Err(api::unknown());
    }
    let grant = delivery.grant.ok_or_else(api::unknown)?;
    if &grant.upload_id != upload_id {
        return Err(api::unknown());
    }
    check_owner(auth, owner).await.map_err(|_| changed())?;
    let index = stages::evidence_index(batch, request, history).map_err(|_| api::unknown())?;
    match upload::put(
        &grant,
        &batch.evidence[index],
        bytes.ok_or_else(api::unknown)?,
    )
    .await
    {
        Ok(()) => Ok(Receipt::Uploaded {
            sha256: sha256.clone(),
        }),
        Err(e) if e.code == "LEARN_UPLOAD_EXPIRED" => Ok(Receipt::GrantExpired {
            upload_id: upload_id.clone(),
        }),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn learn_advance_practice_sync(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    batch_id: String,
) -> Result<BatchView, String> {
    let owner = user(&auth).await?;
    let _write = review_gate::WRITE_GATE.lock().await;
    check_owner(&auth, owner).await?;
    let snapshot = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        journal::load(&db, owner, &batch_id)?
    };
    let Some(request) = snapshot.pending.clone() else {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        return view(&db, owner, snapshot);
    };
    permitted(&auth, &local, &workbench, owner, &snapshot.frozen).await?;
    let (base, token) = super::super::super::courses::context(&auth)
        .await
        .map_err(|e| e.machine_code.to_owned())?;
    check_owner(&auth, owner).await?;
    let (history, bytes) = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        let history = journal::history(&db, owner, &batch_id)?;
        let bytes = if matches!(request.route, Route::Upload { .. }) {
            Some(evidence_bytes(
                &db,
                owner,
                &snapshot.frozen,
                stages::evidence_index(&snapshot.frozen, &request, &history)?,
            )?)
        } else {
            None
        };
        (history, bytes)
    };
    let outcome = send_stage(
        &auth,
        owner,
        &base,
        &token,
        StageInput {
            request: &request,
            batch: &snapshot.frozen,
            history: &history,
            bytes,
        },
    )
    .await;
    check_owner(&auth, owner).await?;
    let result = {
        let mut db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        match outcome {
            Ok(receipt) => match stages::accept(&mut db, owner, &batch_id, &request, &receipt) {
                Ok(snapshot) => view(&db, owner, snapshot)?,
                Err(error) => return Err(error),
            },
            Err(error) => {
                journal::record_error(
                    &mut db,
                    owner,
                    &batch_id,
                    &request,
                    error.code,
                    error.retryable,
                )?;
                view(&db, owner, journal::load(&db, owner, &batch_id)?)?
            }
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn views_are_local_and_filtered_by_full_owner_and_course_scope() {
        let (mut db, course, selection) = super::super::tests::fixture();
        let s = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        let scope = scope(&s.frozen);
        let views = list(&db, 7, &scope).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "pending");
        assert_eq!(views[0].verified_count, 0);
        assert_eq!(views[0].attempt_id, None);
        assert!(list(&db, 8, &scope).unwrap().is_empty());
        for field in [
            "enrollmentId",
            "releaseId",
            "manifestSha256",
            "profileId",
            "unitId",
        ] {
            let mut value = serde_json::to_value(&scope).unwrap();
            value[field] = json!("other");
            assert!(
                list(&db, 7, &serde_json::from_value(value).unwrap())
                    .unwrap()
                    .is_empty(),
                "mixed {field}"
            );
        }
    }
    #[test]
    fn durable_conflict_keeps_original_request_and_success_clears_only_current_error() {
        let (mut db, course, selection) = super::super::tests::fixture();
        let s = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        let request = s.pending.unwrap();
        journal::record_error(
            &mut db,
            7,
            "batch_one",
            &request,
            "LEARN_SYNC_CONFLICT",
            false,
        )
        .unwrap();
        let v = view(&db, 7, journal::load(&db, 7, "batch_one").unwrap()).unwrap();
        assert_eq!(v.status, "conflict");
        assert!(!v.retryable);
        assert_eq!(v.error_code.as_deref(), Some("LEARN_SYNC_CONFLICT"));
        assert_eq!(
            journal::load(&db, 7, "batch_one").unwrap().pending,
            Some(request.clone())
        );
        let next = stages::accept(
            &mut db,
            7,
            "batch_one",
            &request,
            &Receipt::Session {
                session_id: "session_one".into(),
                attempt_id: "attempt_one".into(),
                revision: 1,
            },
        )
        .unwrap();
        assert!(journal::record_error(
            &mut db,
            7,
            "batch_one",
            &request,
            "LEARN_WRITE_UNCONFIRMED",
            true
        )
        .is_err());
        let v = view(&db, 7, next).unwrap();
        assert_eq!(v.status, "pending");
        assert_eq!(v.error_code, None);
    }
}

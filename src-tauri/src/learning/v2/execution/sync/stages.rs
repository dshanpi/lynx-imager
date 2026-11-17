//! Server receipts advance an existing journal; they never re-run a local action.
use super::*;
use journal::{Receipt, Snapshot};

const INVALID: &str = "LEARN_SYNC_RECEIPT_INVALID";
fn register_body(item: &journal::EvidenceRef) -> Value {
    json!({"runId":item.run_id,"kind":item.kind,"sha256":item.sha256,"size":item.size,"scope":"uploaded","producer":"lynx-project-evidence/1","observedAt":item.observed_at})
}
fn registration_index(batch: &FrozenBatch, request: &Request) -> Result<usize, String> {
    let body: Value = serde_json::from_str(&request.body).map_err(|_| INVALID)?;
    batch
        .evidence
        .iter()
        .position(|item| register_body(item) == body)
        .ok_or_else(|| INVALID.into())
}
pub(super) fn evidence_index(
    batch: &FrozenBatch,
    request: &Request,
    history: &[(Request, Option<Receipt>)],
) -> Result<usize, String> {
    let id = match &request.route {
        Route::Evidence { .. } => return registration_index(batch, request),
        Route::Grant { evidence_id }
        | Route::Upload { evidence_id, .. }
        | Route::Completion { evidence_id } => evidence_id,
        _ => return Err(INVALID.into()),
    };
    let registrations: Vec<_> = history
        .iter()
        .filter(|(_, ack)| matches!(ack,Some(Receipt::Evidence{evidence_id,..}) if evidence_id==id))
        .collect();
    if registrations.len() != 1 {
        return Err(INVALID.into());
    }
    registration_index(batch, &registrations[0].0)
}
fn next_request(
    current: &Request,
    batch: &FrozenBatch,
    route: Route,
    body: Value,
) -> Result<Request, String> {
    let sequence = current.sequence.checked_add(1).ok_or(INVALID)?;
    let seed = hex::encode(Sha256::digest(batch.batch_id.as_bytes()));
    Request::new(
        sequence,
        format!("sync_{}_{sequence}", &seed[..32]),
        route,
        body,
        None,
    )
}
pub(super) fn next(
    batch: &FrozenBatch,
    current: &Request,
    receipt: &Receipt,
    history: &[(Request, Option<Receipt>)],
) -> Result<Option<Request>, String> {
    let attempt_id = history.first().and_then(|(_, ack)| match ack {
        Some(Receipt::Session { attempt_id, .. }) => Some(attempt_id.clone()),
        _ => None,
    });
    let request = match (&current.route, receipt) {
        (
            Route::Session,
            Receipt::Session {
                attempt_id,
                revision,
                ..
            },
        ) => {
            if *revision != 1 {
                return Err(INVALID.into());
            }
            next_request(
                current,
                batch,
                Route::Events {
                    attempt_id: attempt_id.clone(),
                },
                serde_json::from_str(&batch.event_body).map_err(|_| INVALID)?,
            )?
        }
        (
            Route::Events { attempt_id: actual },
            Receipt::Events {
                revision,
                last_sequence,
                accepted_ids,
                duplicate_ids,
            },
        ) => {
            if Some(actual) != attempt_id.as_ref() || current.body != batch.event_body {
                return Err(INVALID.into());
            }
            let events: Value = serde_json::from_str(&current.body).map_err(|_| INVALID)?;
            let expected = events["events"].as_array().ok_or(INVALID)?;
            let expected: std::collections::HashSet<_> = expected
                .iter()
                .map(|v| v["eventId"].as_str().ok_or(INVALID))
                .collect::<Result<_, _>>()?;
            let received: std::collections::HashSet<_> = accepted_ids
                .iter()
                .chain(duplicate_ids)
                .map(String::as_str)
                .collect();
            if *revision != 2
                || *last_sequence != expected.len() as u64
                || received != expected
                || received.len() != accepted_ids.len() + duplicate_ids.len()
            {
                return Err(INVALID.into());
            }
            next_request(
                current,
                batch,
                Route::Evidence {
                    attempt_id: actual.clone(),
                },
                register_body(batch.evidence.first().ok_or(INVALID)?),
            )?
        }
        (Route::Evidence { attempt_id: actual }, Receipt::Evidence { evidence_id, .. }) => {
            if Some(actual)!=attempt_id.as_ref() || history.iter().any(|(_,ack)|matches!(ack,Some(Receipt::Evidence{evidence_id:id,..}) if id==evidence_id)) {return Err(INVALID.into());}
            let item = &batch.evidence[registration_index(batch, current)?];
            next_request(
                current,
                batch,
                Route::Grant {
                    evidence_id: evidence_id.clone(),
                },
                json!({"contentType":item.content_type,"size":item.size,"sha256":item.sha256}),
            )?
        }
        (Route::Grant { evidence_id }, Receipt::Grant { upload_id, .. }) => {
            let item = &batch.evidence[evidence_index(batch, current, history)?];
            if history.iter().any(
                |(_, ack)| matches!(ack,Some(Receipt::Grant{upload_id:id,..}) if id==upload_id),
            ) {
                return Err(INVALID.into());
            }
            next_request(
                current,
                batch,
                Route::Upload {
                    evidence_id: evidence_id.clone(),
                    upload_id: upload_id.clone(),
                    run_id: item.run_id.clone(),
                    evidence_kind: item.kind.clone(),
                    sha256: item.sha256.clone(),
                },
                json!({}),
            )?
        }
        (
            Route::Upload {
                evidence_id,
                upload_id,
                sha256,
                ..
            },
            Receipt::Uploaded { sha256: uploaded },
        ) => {
            let item = &batch.evidence[evidence_index(batch, current, history)?];
            if uploaded != sha256 || sha256 != &item.sha256 {
                return Err(INVALID.into());
            }
            next_request(
                current,
                batch,
                Route::Completion {
                    evidence_id: evidence_id.clone(),
                },
                json!({"uploadId":upload_id}),
            )?
        }
        (
            Route::Upload { evidence_id, .. } | Route::Completion { evidence_id },
            Receipt::GrantExpired { .. },
        ) => {
            let item = &batch.evidence[evidence_index(batch, current, history)?];
            next_request(
                current,
                batch,
                Route::Grant {
                    evidence_id: evidence_id.clone(),
                },
                json!({"contentType":item.content_type,"size":item.size,"sha256":item.sha256}),
            )?
        }
        (Route::Completion { evidence_id }, Receipt::UploadIncomplete { upload_id }) => {
            let prior = history
                .iter()
                .rev()
                .find_map(|(r, _)| match &r.route {
                    Route::Upload {
                        evidence_id: id,
                        upload_id: upload,
                        ..
                    } if id == evidence_id && upload == upload_id => Some(r),
                    _ => None,
                })
                .ok_or(INVALID)?;
            next_request(current, batch, prior.route.clone(), json!({}))?
        }
        (
            Route::Completion { evidence_id },
            Receipt::Completion {
                evidence_id: received,
                verification_status,
                trust_level,
            },
        ) => {
            if evidence_id != received
                || verification_status != "file_verified"
                || trust_level != "user_asserted"
            {
                return Err(INVALID.into());
            }
            let index = evidence_index(batch, current, history)?;
            let Some(item) = batch.evidence.get(index + 1) else {
                return Ok(None);
            };
            next_request(
                current,
                batch,
                Route::Evidence {
                    attempt_id: attempt_id.ok_or(INVALID)?,
                },
                register_body(item),
            )?
        }
        _ => return Err(INVALID.into()),
    };
    Ok(Some(request))
}
pub(super) fn accept(
    db: &mut Connection,
    owner: i64,
    batch_id: &str,
    current: &Request,
    receipt: &Receipt,
) -> Result<Snapshot, String> {
    let snapshot = journal::load(db, owner, batch_id)?;
    if snapshot.pending.as_ref() != Some(current) {
        return Err("LEARN_SYNC_CONFLICT".into());
    }
    let history = journal::history(db, owner, batch_id)?;
    let next = next(&snapshot.frozen, current, receipt, &history)?;
    journal::acknowledge(db, owner, batch_id, current, receipt, next.as_ref())?;
    journal::load(db, owner, batch_id)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    fn start() -> (Connection, Snapshot) {
        let (mut db, course, selection) = super::super::tests::fixture();
        let snapshot = prepare(&mut db, 7, &selection, &course, &"b".repeat(64)).unwrap();
        (db, snapshot)
    }
    pub(in super::super) fn response(request: &Request, batch: &FrozenBatch) -> Receipt {
        match &request.route {
            Route::Session => Receipt::Session {
                session_id: "session_one".into(),
                attempt_id: "attempt_one".into(),
                revision: 1,
            },
            Route::Events { .. } => {
                let b: Value = serde_json::from_str(&batch.event_body).unwrap();
                let ids = b["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["eventId"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                Receipt::Events {
                    revision: 2,
                    last_sequence: ids.len() as u64,
                    accepted_ids: ids,
                    duplicate_ids: vec![],
                }
            }
            Route::Evidence { .. } => Receipt::Evidence {
                evidence_id: format!("evidence_{}", request.sequence),
                verification_status: "metadata_received".into(),
                trust_level: "user_asserted".into(),
            },
            Route::Grant { .. } => Receipt::Grant {
                upload_id: format!("upload_{}", request.sequence),
                expires_at: "2026-10-18T12:00:00Z".into(),
            },
            Route::Upload { sha256, .. } => Receipt::Uploaded {
                sha256: sha256.clone(),
            },
            Route::Completion { evidence_id } => Receipt::Completion {
                evidence_id: evidence_id.clone(),
                verification_status: "file_verified".into(),
                trust_level: "user_asserted".into(),
            },
        }
    }
    #[test]
    fn complete_all_evidence_then_done_never_creates_an_assessment() {
        let (mut db, mut snapshot) = start();
        let mut completed = 0;
        while let Some(request) = snapshot.pending.clone() {
            let ack = response(&request, &snapshot.frozen);
            if let Receipt::Completion { .. } = ack {
                completed += 1;
            }
            snapshot = accept(&mut db, 7, "batch_one", &request, &ack).unwrap();
            let restored = journal::load(&db, 7, "batch_one").unwrap();
            assert_eq!(snapshot.pending, restored.pending);
        }
        assert_eq!(completed, 3);
        assert_eq!(snapshot.revision, 13);
        assert_eq!(journal::history(&db, 7, "batch_one").unwrap().len(), 14);
        let first = journal::history(&db, 7, "batch_one").unwrap()[0].0.clone();
        assert!(accept(
            &mut db,
            7,
            "batch_one",
            &first,
            &response(&first, &snapshot.frozen)
        )
        .is_err());
    }
    #[test]
    fn incomplete_event_receipts_do_not_register_or_advance() {
        let (mut db, s) = start();
        let first = s.pending.unwrap();
        let s = accept(
            &mut db,
            7,
            "batch_one",
            &first,
            &response(&first, &s.frozen),
        )
        .unwrap();
        let request = s.pending.unwrap();
        let mut ack = response(&request, &s.frozen);
        if let Receipt::Events { accepted_ids, .. } = &mut ack {
            accepted_ids.pop();
        }
        assert!(accept(&mut db, 7, "batch_one", &request, &ack).is_err());
        assert_eq!(
            journal::load(&db, 7, "batch_one").unwrap().pending,
            Some(request.clone())
        );
        let mut ack = response(&request, &s.frozen);
        if let Receipt::Events {
            accepted_ids,
            duplicate_ids,
            ..
        } = &mut ack
        {
            duplicate_ids.push(accepted_ids[0].clone());
        }
        assert!(accept(&mut db, 7, "batch_one", &request, &ack).is_err());
    }
    #[test]
    fn expired_grants_keep_same_evidence_and_completion_intent_is_not_skipped() {
        let (mut db, mut s) = start();
        while !matches!(s.pending.as_ref().unwrap().route, Route::Upload { .. }) {
            let r = s.pending.clone().unwrap();
            s = accept(&mut db, 7, "batch_one", &r, &response(&r, &s.frozen)).unwrap();
        }
        let old = s.pending.clone().unwrap();
        let Route::Upload {
            upload_id,
            evidence_id,
            ..
        } = &old.route
        else {
            panic!()
        };
        let renew = accept(
            &mut db,
            7,
            "batch_one",
            &old,
            &Receipt::GrantExpired {
                upload_id: upload_id.clone(),
            },
        )
        .unwrap();
        let grant = renew.pending.unwrap();
        assert_eq!(
            grant.route,
            Route::Grant {
                evidence_id: evidence_id.clone()
            }
        );
        assert_ne!(old.key, grant.key);
        s = accept(
            &mut db,
            7,
            "batch_one",
            &grant,
            &response(&grant, &s.frozen),
        )
        .unwrap();
        let upload = s.pending.clone().unwrap();
        s = accept(
            &mut db,
            7,
            "batch_one",
            &upload,
            &response(&upload, &s.frozen),
        )
        .unwrap();
        let completion = s.pending.unwrap();
        assert!(matches!(completion.route, Route::Completion { .. }));
        // A restart keeps completion first. Mere local expiry cannot replace it.
        assert_eq!(
            journal::load(&db, 7, "batch_one").unwrap().pending,
            Some(completion.clone())
        );
        assert!(accept(
            &mut db,
            7,
            "batch_one",
            &completion,
            &Receipt::GrantExpired {
                upload_id: "wrong".into()
            }
        )
        .is_err());
        assert_eq!(
            journal::load(&db, 7, "batch_one").unwrap().pending,
            Some(completion)
        );
    }
    #[test]
    fn wrong_evidence_or_promoted_trust_is_not_an_accepted_completion() {
        let (mut db, mut s) = start();
        while !matches!(s.pending.as_ref().unwrap().route, Route::Completion { .. }) {
            let r = s.pending.clone().unwrap();
            s = accept(&mut db, 7, "batch_one", &r, &response(&r, &s.frozen)).unwrap();
        }
        let r = s.pending.unwrap();
        for trust in ["client_measured", "device_attested", "server_verified"] {
            let Route::Completion { evidence_id } = &r.route else {
                panic!()
            };
            assert!(accept(
                &mut db,
                7,
                "batch_one",
                &r,
                &Receipt::Completion {
                    evidence_id: evidence_id.clone(),
                    verification_status: "file_verified".into(),
                    trust_level: trust.into()
                }
            )
            .is_err());
        }
        assert!(accept(
            &mut db,
            7,
            "batch_one",
            &r,
            &Receipt::Completion {
                evidence_id: "another".into(),
                verification_status: "file_verified".into(),
                trust_level: "user_asserted".into()
            }
        )
        .is_err());
    }
    #[test]
    fn explicit_incomplete_receipt_retries_same_object_before_completion() {
        let (mut db, mut s) = start();
        let mut original = None;
        while !matches!(s.pending.as_ref().unwrap().route, Route::Completion { .. }) {
            let r = s.pending.clone().unwrap();
            if matches!(r.route, Route::Upload { .. }) {
                original = Some(r.route.clone());
            }
            s = accept(&mut db, 7, "batch_one", &r, &response(&r, &s.frozen)).unwrap();
        }
        let current = s.pending.unwrap();
        let body: Value = serde_json::from_str(&current.body).unwrap();
        let next = accept(
            &mut db,
            7,
            "batch_one",
            &current,
            &Receipt::UploadIncomplete {
                upload_id: body["uploadId"].as_str().unwrap().into(),
            },
        )
        .unwrap()
        .pending
        .unwrap();
        assert_eq!(Some(next.route), original);
        assert_ne!(next.key, current.key);
    }
}

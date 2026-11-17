//! Atomic run ownership and immutable request journal; no credentials or upload URLs.
use super::super::super::manifest;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const INVALID: &str = "LEARN_SYNC_JOURNAL_INVALID";
const STORE: &str = "LEARN_STORE_UNAVAILABLE";
const CONFLICT: &str = "LEARN_SYNC_CONFLICT";
const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_STEPS: u32 = 512;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRef {
    pub run_id: String,
    pub snapshot_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRef {
    pub run_id: String,
    pub kind: String,
    pub sha256: String,
    pub size: usize,
    pub content_type: String,
    pub observed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrozenBatch {
    pub batch_id: String,
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub unit_id: String,
    pub profile_id: String,
    pub binding_digest: String,
    pub context_fingerprint: String,
    pub instance_id: String,
    pub runs: Vec<RunRef>,
    pub evidence: Vec<EvidenceRef>,
    /// Exact ordered event payload, frozen before the first network request.
    pub event_body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Route {
    Session,
    Events {
        attempt_id: String,
    },
    Evidence {
        attempt_id: String,
    },
    Grant {
        evidence_id: String,
    },
    Upload {
        evidence_id: String,
        upload_id: String,
        run_id: String,
        evidence_kind: String,
        sha256: String,
    },
    Completion {
        evidence_id: String,
    },
}
impl Route {
    fn valid(&self) -> bool {
        match self {
            Self::Session => true,
            Self::Events { attempt_id } | Self::Evidence { attempt_id } => {
                manifest::valid_id(attempt_id)
            }
            Self::Grant { evidence_id } | Self::Completion { evidence_id } => {
                manifest::valid_id(evidence_id)
            }
            Self::Upload {
                evidence_id,
                upload_id,
                run_id,
                evidence_kind,
                sha256,
            } => {
                [evidence_id, upload_id, run_id]
                    .iter()
                    .all(|s| manifest::valid_id(s))
                    && ["stdout", "build_log", "exit_status"].contains(&evidence_kind.as_str())
                    && manifest::valid_digest(sha256)
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub sequence: u32,
    pub key: String,
    pub route: Route,
    pub body: String,
    pub if_match: Option<u64>,
}
impl Request {
    pub fn new(
        sequence: u32,
        key: String,
        route: Route,
        body: Value,
        if_match: Option<u64>,
    ) -> Result<Self, String> {
        let request = Self {
            sequence,
            key,
            route,
            body: body.to_string(),
            if_match,
        };
        request.validate()?;
        Ok(request)
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.sequence >= MAX_STEPS
            || !manifest::valid_id(&self.key)
            || !self.route.valid()
            || self.body.len() > MAX_REQUEST_BYTES
            || self.if_match.is_some()
        {
            return Err(INVALID.into());
        }
        let body: Value = serde_json::from_str(&self.body).map_err(|_| INVALID)?;
        let fields: &[&str] = match self.route {
            Route::Session => &["enrollmentId", "unitId", "instanceId"],
            Route::Events { .. } => &["baseRevision", "instanceId", "events"],
            Route::Evidence { .. } => &[
                "runId",
                "kind",
                "sha256",
                "size",
                "scope",
                "producer",
                "observedAt",
            ],
            Route::Grant { .. } => &["contentType", "size", "sha256"],
            Route::Upload { .. } => &[],
            Route::Completion { .. } => &["uploadId"],
        };
        let object = body.as_object().ok_or(INVALID)?;
        if object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str()))
        {
            return Err(INVALID.into());
        }
        Ok(())
    }
}

/// Only these non-secret fields may be journaled. A signed URL is never a receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Receipt {
    Session {
        session_id: String,
        attempt_id: String,
        revision: u64,
    },
    Events {
        revision: u64,
        last_sequence: u64,
        accepted_ids: Vec<String>,
        duplicate_ids: Vec<String>,
    },
    Evidence {
        evidence_id: String,
        verification_status: String,
        trust_level: String,
    },
    Grant {
        upload_id: String,
        expires_at: String,
    },
    Uploaded {
        sha256: String,
    },
    Completion {
        evidence_id: String,
        verification_status: String,
        trust_level: String,
    },
    GrantExpired {
        upload_id: String,
    },
    UploadIncomplete {
        upload_id: String,
    },
}

#[derive(Debug)]
pub struct Snapshot {
    pub frozen: FrozenBatch,
    pub revision: u32,
    pub pending: Option<Request>,
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_sync_batches(batch_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL,frozen TEXT NOT NULL,frozen_sha256 TEXT NOT NULL,revision INTEGER NOT NULL,done INTEGER NOT NULL CHECK(done IN (0,1)));
        CREATE INDEX IF NOT EXISTS learn_sync_owner ON learn_sync_batches(user_id,batch_id);
        CREATE TABLE IF NOT EXISTS learn_sync_run_claims(run_id TEXT PRIMARY KEY,batch_id TEXT NOT NULL,user_id INTEGER NOT NULL,snapshot_sha256 TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS learn_sync_requests(batch_id TEXT NOT NULL,sequence INTEGER NOT NULL,user_id INTEGER NOT NULL,request_key TEXT NOT NULL,request TEXT NOT NULL,request_sha256 TEXT NOT NULL,receipt TEXT,receipt_sha256 TEXT,PRIMARY KEY(batch_id,sequence),UNIQUE(user_id,request_key));
        CREATE TABLE IF NOT EXISTS learn_sync_errors(batch_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL,sequence INTEGER NOT NULL,code TEXT NOT NULL,retryable INTEGER NOT NULL CHECK(retryable IN (0,1)));")
}
fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| INVALID.into())
}
fn sha(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn decode<T: serde::de::DeserializeOwned>(raw: &str, digest: &str) -> Result<T, String> {
    if raw.len() > 2 * MAX_REQUEST_BYTES || sha(raw) != digest {
        return Err(INVALID.into());
    }
    serde_json::from_str(raw).map_err(|_| INVALID.into())
}
pub(super) fn validate_batch(batch: &FrozenBatch) -> Result<(), String> {
    if [
        &batch.batch_id,
        &batch.enrollment_id,
        &batch.release_id,
        &batch.unit_id,
        &batch.profile_id,
        &batch.instance_id,
    ]
    .iter()
    .any(|s| !manifest::valid_id(s))
        || [
            &batch.manifest_sha256,
            &batch.binding_digest,
            &batch.context_fingerprint,
        ]
        .iter()
        .any(|s| !manifest::valid_digest(s))
        || batch.runs.is_empty()
        || batch.runs.len() > 8
        || batch.event_body.len() > MAX_REQUEST_BYTES
    {
        return Err(INVALID.into());
    }
    let mut ids = std::collections::HashSet::new();
    for run in &batch.runs {
        if !manifest::valid_id(&run.run_id)
            || !manifest::valid_digest(&run.snapshot_sha256)
            || !ids.insert(&run.run_id)
        {
            return Err(INVALID.into());
        }
    }
    if batch.evidence.is_empty() || batch.evidence.len() > 24 {
        return Err(INVALID.into());
    }
    let mut evidence_ids = std::collections::HashSet::new();
    let mut bytes = 0usize;
    for item in &batch.evidence {
        bytes = bytes.checked_add(item.size).ok_or(INVALID)?;
        if !ids.contains(&item.run_id)
            || !["stdout", "build_log", "exit_status"].contains(&item.kind.as_str())
            || !manifest::valid_digest(&item.sha256)
            || item.size > 4 * 1024 * 1024
            || bytes > 16 * 1024 * 1024
            || item.content_type
                != if item.kind == "exit_status" {
                    "application/json"
                } else {
                    "text/plain"
                }
            || item.observed_at.len() > 64
            || chrono::DateTime::parse_from_rfc3339(&item.observed_at).is_err()
            || !evidence_ids.insert((&item.run_id, &item.kind))
        {
            return Err(INVALID.into());
        }
    }
    let body: Value = serde_json::from_str(&batch.event_body).map_err(|_| INVALID)?;
    if body["baseRevision"] != 1
        || body["instanceId"] != batch.instance_id
        || !body["events"].is_array()
    {
        return Err(INVALID.into());
    }
    Ok(())
}
fn enqueue(db: &Connection, owner: i64, batch: &str, request: &Request) -> Result<(), String> {
    request.validate()?;
    let raw = encode(request)?;
    if raw.len() > 2 * MAX_REQUEST_BYTES {
        return Err(INVALID.into());
    }
    db.execute("INSERT INTO learn_sync_requests(batch_id,sequence,user_id,request_key,request,request_sha256) VALUES(?1,?2,?3,?4,?5,?6)",params![batch,request.sequence,owner,request.key,raw,sha(&raw)]).map_err(|_|STORE)?;
    Ok(())
}
#[cfg(test)]
pub fn create(
    db: &mut Connection,
    owner: i64,
    batch: &FrozenBatch,
    request: &Request,
) -> Result<(), String> {
    let tx = db.transaction().map_err(|_| STORE)?;
    create_in_transaction(&tx, owner, batch, request)?;
    tx.commit().map_err(|_| STORE.into())
}
pub(super) fn create_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    owner: i64,
    batch: &FrozenBatch,
    request: &Request,
) -> Result<(), String> {
    validate_batch(batch)?;
    request.validate()?;
    let first = serde_json::json!({"enrollmentId":batch.enrollment_id,"unitId":batch.unit_id,"instanceId":batch.instance_id}).to_string();
    if owner <= 0
        || request.sequence != 0
        || request.route != Route::Session
        || request.body != first
    {
        return Err(INVALID.into());
    }
    let existing: Option<i64> = tx
        .query_row(
            "SELECT user_id FROM learn_sync_batches WHERE batch_id=?1",
            [&batch.batch_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| STORE)?;
    if let Some(existing) = existing {
        if existing != owner
            || load(tx, owner, &batch.batch_id)?.frozen != *batch
            || read_request(tx, owner, &batch.batch_id, 0)?.0 != *request
        {
            return Err(CONFLICT.into());
        }
        return Ok(());
    }
    let raw = encode(batch)?;
    if raw.len() > 2 * MAX_REQUEST_BYTES {
        return Err(INVALID.into());
    }
    tx.execute(
        "INSERT INTO learn_sync_batches VALUES(?1,?2,?3,?4,0,0)",
        params![batch.batch_id, owner, raw, sha(&raw)],
    )
    .map_err(|_| STORE)?;
    for run in &batch.runs {
        tx.execute(
            "INSERT INTO learn_sync_run_claims VALUES(?1,?2,?3,?4)",
            params![run.run_id, batch.batch_id, owner, run.snapshot_sha256],
        )
        .map_err(|_| "LEARN_SYNC_RUN_ALREADY_CLAIMED")?;
    }
    enqueue(tx, owner, &batch.batch_id, request)?;
    Ok(())
}
fn read_request(
    db: &Connection,
    owner: i64,
    batch: &str,
    sequence: u32,
) -> Result<(Request, Option<Receipt>), String> {
    let (raw,digest,receipt,receipt_sha,key):(String,String,Option<String>,Option<String>,String)=db.query_row("SELECT request,request_sha256,receipt,receipt_sha256,request_key FROM learn_sync_requests WHERE user_id=?1 AND batch_id=?2 AND sequence=?3",params![owner,batch,sequence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|INVALID)?;
    let request: Request = decode(&raw, &digest)?;
    request.validate()?;
    if request.sequence != sequence || request.key != key {
        return Err(INVALID.into());
    }
    let receipt = match (receipt, receipt_sha) {
        (None, None) => None,
        (Some(r), Some(s)) => Some(decode(&r, &s)?),
        _ => return Err(INVALID.into()),
    };
    Ok((request, receipt))
}
#[cfg(test)]
pub fn receipt(
    db: &Connection,
    owner: i64,
    batch: &str,
    sequence: u32,
) -> Result<Option<Receipt>, String> {
    load(db, owner, batch)?;
    Ok(read_request(db, owner, batch, sequence)?.1)
}
pub fn history(
    db: &Connection,
    owner: i64,
    batch: &str,
) -> Result<Vec<(Request, Option<Receipt>)>, String> {
    let snapshot = load(db, owner, batch)?;
    (0..=snapshot.revision)
        .map(|sequence| read_request(db, owner, batch, sequence))
        .collect()
}
pub fn record_error(
    db: &mut Connection,
    owner: i64,
    batch: &str,
    request: &Request,
    code: &'static str,
    retryable: bool,
) -> Result<(), String> {
    if code.len() > 64
        || !code.starts_with("LEARN_")
        || !code.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')
    {
        return Err(INVALID.into());
    }
    let tx = db.transaction().map_err(|_| STORE)?;
    if load(&tx, owner, batch)?.pending.as_ref() != Some(request) {
        return Err(CONFLICT.into());
    }
    tx.execute("INSERT INTO learn_sync_errors VALUES(?1,?2,?3,?4,?5) ON CONFLICT(batch_id) DO UPDATE SET sequence=excluded.sequence,code=excluded.code,retryable=excluded.retryable WHERE learn_sync_errors.user_id=excluded.user_id",params![batch,owner,request.sequence,code,retryable]).map_err(|_|STORE)?;
    tx.commit().map_err(|_| STORE.into())
}
pub fn last_error(
    db: &Connection,
    owner: i64,
    batch: &str,
    sequence: u32,
) -> Result<Option<(String, bool)>, String> {
    db.query_row("SELECT code,retryable FROM learn_sync_errors WHERE user_id=?1 AND batch_id=?2 AND sequence=?3",params![owner,batch,sequence],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|STORE.into())
}
pub fn load(db: &Connection, owner: i64, batch: &str) -> Result<Snapshot, String> {
    let (raw,digest,revision,done):(String,String,u32,bool)=db.query_row("SELECT frozen,frozen_sha256,revision,done FROM learn_sync_batches WHERE user_id=?1 AND batch_id=?2",params![owner,batch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"LEARN_SYNC_NOT_FOUND")?;
    let frozen: FrozenBatch = decode(&raw, &digest)?;
    validate_batch(&frozen)?;
    if frozen.batch_id != batch || revision >= MAX_STEPS {
        return Err(INVALID.into());
    }
    let claims: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM learn_sync_run_claims WHERE batch_id=?1",
            [batch],
            |r| r.get(0),
        )
        .map_err(|_| STORE)?;
    if claims != frozen.runs.len() as i64 {
        return Err(INVALID.into());
    }
    for run in &frozen.runs {
        let claim:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM learn_sync_run_claims WHERE user_id=?1 AND batch_id=?2 AND run_id=?3 AND snapshot_sha256=?4)",params![owner,batch,run.run_id,run.snapshot_sha256],|r|r.get(0)).map_err(|_|STORE)?;
        if !claim {
            return Err(INVALID.into());
        }
    }
    let count: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM learn_sync_requests WHERE batch_id=?1",
            [batch],
            |r| r.get(0),
        )
        .map_err(|_| STORE)?;
    if count != revision + 1 {
        return Err(INVALID.into());
    }
    let mut pending = None;
    for sequence in 0..=revision {
        let (request, ack) = read_request(db, owner, batch, sequence)?;
        if sequence == revision && !done {
            if ack.is_some() {
                return Err(INVALID.into());
            }
            pending = Some(request);
        } else {
            let ack = ack.ok_or(INVALID)?;
            let next = if sequence < revision {
                Some(read_request(db, owner, batch, sequence + 1)?.0)
            } else {
                None
            };
            transition(&request, &ack, next.as_ref())?;
        }
    }
    Ok(Snapshot {
        frozen,
        revision,
        pending,
    })
}
fn transition(current: &Request, receipt: &Receipt, next: Option<&Request>) -> Result<(), String> {
    if let Some(next) = next {
        next.validate()?;
        if next.sequence != current.sequence + 1 || next.key == current.key {
            return Err(INVALID.into());
        }
    }
    let next_route = next.map(|n| &n.route);
    let valid = match (&current.route, receipt, next_route) {
        (
            Route::Session,
            Receipt::Session {
                session_id,
                attempt_id,
                revision,
            },
            Some(Route::Events {
                attempt_id: next_id,
            }),
        ) => {
            *revision == 1
                && manifest::valid_id(session_id)
                && manifest::valid_id(attempt_id)
                && attempt_id == next_id
        }
        (
            Route::Events { attempt_id },
            Receipt::Events {
                revision,
                last_sequence,
                accepted_ids,
                duplicate_ids,
            },
            Some(Route::Evidence {
                attempt_id: next_id,
            }),
        ) => {
            *revision >= 2
                && *last_sequence > 0
                && attempt_id == next_id
                && accepted_ids.len() + duplicate_ids.len() <= 32
                && accepted_ids
                    .iter()
                    .chain(duplicate_ids)
                    .all(|id| manifest::valid_id(id))
        }
        (
            Route::Evidence { .. },
            Receipt::Evidence {
                evidence_id,
                verification_status,
                trust_level,
            },
            Some(Route::Grant {
                evidence_id: next_id,
            }),
        ) => {
            manifest::valid_id(evidence_id)
                && evidence_id == next_id
                && verification_status == "metadata_received"
                && trust_level == "user_asserted"
        }
        (
            Route::Grant { evidence_id },
            Receipt::Grant {
                upload_id,
                expires_at,
            },
            Some(Route::Upload {
                evidence_id: next_id,
                upload_id: next_upload,
                ..
            }),
        ) => {
            manifest::valid_id(upload_id)
                && evidence_id == next_id
                && upload_id == next_upload
                && expires_at.len() <= 64
                && chrono::DateTime::parse_from_rfc3339(expires_at).is_ok()
        }
        (
            Route::Upload {
                evidence_id,
                upload_id,
                sha256,
                ..
            },
            Receipt::Uploaded { sha256: uploaded },
            Some(Route::Completion {
                evidence_id: next_id,
            }),
        ) => {
            sha256 == uploaded
                && evidence_id == next_id
                && next.is_some_and(|n| {
                    serde_json::from_str::<Value>(&n.body)
                        .ok()
                        .is_some_and(|v| v["uploadId"] == *upload_id)
                })
        }
        (
            Route::Upload {
                evidence_id,
                upload_id,
                ..
            },
            Receipt::GrantExpired { upload_id: expired },
            Some(Route::Grant {
                evidence_id: next_id,
            }),
        ) => upload_id == expired && evidence_id == next_id,
        (
            Route::Completion { evidence_id },
            Receipt::GrantExpired { upload_id },
            Some(Route::Grant {
                evidence_id: next_id,
            }),
        ) => {
            evidence_id == next_id
                && serde_json::from_str::<Value>(&current.body)
                    .ok()
                    .is_some_and(|v| v["uploadId"] == *upload_id)
        }
        (
            Route::Completion { evidence_id },
            Receipt::UploadIncomplete { upload_id },
            Some(Route::Upload {
                evidence_id: next_id,
                upload_id: next_upload,
                ..
            }),
        ) => {
            evidence_id == next_id
                && upload_id == next_upload
                && serde_json::from_str::<Value>(&current.body)
                    .ok()
                    .is_some_and(|v| v["uploadId"] == *upload_id)
        }
        (
            Route::Completion { evidence_id },
            Receipt::Completion {
                evidence_id: received,
                verification_status,
                trust_level,
            },
            None | Some(Route::Evidence { .. }),
        ) => {
            evidence_id == received
                && verification_status == "file_verified"
                && trust_level == "user_asserted"
        }
        _ => false,
    };
    if !valid {
        return Err(INVALID.into());
    }
    Ok(())
}
pub fn acknowledge(
    db: &mut Connection,
    owner: i64,
    batch: &str,
    current: &Request,
    receipt: &Receipt,
    next: Option<&Request>,
) -> Result<(), String> {
    transition(current, receipt, next)?;
    let tx = db.transaction().map_err(|_| STORE)?;
    let snapshot = load(&tx, owner, batch)?;
    if snapshot.pending.as_ref() != Some(current) {
        return Err(CONFLICT.into());
    }
    let raw = encode(receipt)?;
    let changed=tx.execute("UPDATE learn_sync_requests SET receipt=?1,receipt_sha256=?2 WHERE user_id=?3 AND batch_id=?4 AND sequence=?5 AND receipt IS NULL",params![raw,sha(&raw),owner,batch,current.sequence]).map_err(|_|STORE)?;
    if changed != 1 {
        return Err(CONFLICT.into());
    }
    if let Some(next) = next {
        enqueue(&tx, owner, batch, next)?;
    }
    tx.execute(
        "DELETE FROM learn_sync_errors WHERE user_id=?1 AND batch_id=?2",
        params![owner, batch],
    )
    .map_err(|_| STORE)?;
    tx.execute(
        "UPDATE learn_sync_batches SET revision=?1,done=?2 WHERE user_id=?3 AND batch_id=?4",
        params![
            next.map_or(current.sequence, |n| n.sequence),
            next.is_none(),
            owner,
            batch
        ],
    )
    .map_err(|_| STORE)?;
    tx.commit().map_err(|_| STORE.into())
}

#[cfg(test)]
mod tests;

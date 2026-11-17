//! Durable practice synchronization. Network pumping is explicitly user initiated.
mod api;
pub mod commands;
pub mod freshness;
pub mod journal;
mod plan;
pub mod review_gate;
mod stages;
mod upload;

use super::*;
use journal::{FrozenBatch, Request, Route, RunRef};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub batch_id: String,
    #[serde(flatten)]
    pub environment: environment::EnvironmentRequest,
    pub unit_id: String,
    pub runs: Vec<RunRef>,
}

/// Called only after fresh normal entitlement, signature, account and binding checks.
/// Recheck the actual snapshots and freeze the first request in one transaction.
pub fn prepare(
    db: &mut Connection,
    owner: i64,
    selection: &Selection,
    course: &manifest::VerifiedCourse,
    binding_digest: &str,
) -> Result<journal::Snapshot, String> {
    if owner <= 0
        || !manifest::valid_id(&selection.batch_id)
        || !manifest::valid_id(&selection.unit_id)
        || selection.runs.is_empty()
        || selection.runs.len() > 8
        || !manifest::valid_digest(binding_digest)
    {
        return Err("LEARN_SYNC_SELECTION_INVALID".into());
    }
    let tx = db.transaction().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for selected in &selection.runs {
        if !manifest::valid_id(&selected.run_id)
            || !manifest::valid_digest(&selected.snapshot_sha256)
            || !seen.insert(&selected.run_id)
        {
            return Err("LEARN_SYNC_SELECTION_INVALID".into());
        }
        let raw: String = tx
            .query_row(
                "SELECT record FROM learn_project_runs WHERE user_id=?1 AND run_id=?2",
                params![owner, selected.run_id],
                |r| r.get(0),
            )
            .map_err(|_| "LEARN_RUN_NOT_FOUND")?;
        if raw.len() > 256 * 1024 {
            return Err("LEARN_RUN_INVALID".into());
        }
        let saved: Stored = serde_json::from_str(&raw).map_err(|_| "LEARN_RUN_INVALID")?;
        let expected_scope = hex::encode(Sha256::digest(
            serde_json::to_vec(
                &json!({"user":owner,"request":saved.run.request,"binding":saved.binding_digest}),
            )
            .map_err(|_| "LEARN_RUN_INVALID")?,
        ));
        if saved.scope != expected_scope
            || saved.run.run_id != selected.run_id
            || saved.binding_digest != binding_digest
            || saved.run.request.unit_id != selection.unit_id
            || serde_json::to_value(&saved.run.request.environment)
                .map_err(|_| "LEARN_RUN_INVALID")?
                != serde_json::to_value(&selection.environment).map_err(|_| "LEARN_RUN_INVALID")?
        {
            return Err("LEARN_SYNC_SELECTION_INCOMPATIBLE".into());
        }
        let preview =
            evidence::restore(&tx, owner, &saved)?.ok_or("LEARN_SYNC_PREVIEW_REQUIRED")?;
        if preview.snapshot_sha256 != selected.snapshot_sha256 {
            return Err("LEARN_EVIDENCE_CHANGED".into());
        }
        entries.push((saved, preview));
    }
    let seed = hex::encode(Sha256::digest(selection.batch_id.as_bytes()));
    let instance_id = format!("instance_{}", &seed[..32]);
    let batch = plan::compile(course, &selection.batch_id, &instance_id, &entries)?;
    let request = Request::new(
        0,
        format!("sync_{}_0", &seed[..32]),
        Route::Session,
        json!({"enrollmentId":batch.enrollment_id,"unitId":batch.unit_id,"instanceId":batch.instance_id}),
        None,
    )?;
    journal::create_in_transaction(&tx, owner, &batch, &request)?;
    let result = journal::load(&tx, owner, &selection.batch_id)?;
    tx.commit().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    Ok(result)
}

/// Restore the exact reviewed bytes; never query SSH or recreate old evidence.
pub fn evidence_bytes(
    db: &Connection,
    owner: i64,
    batch: &FrozenBatch,
    index: usize,
) -> Result<Vec<u8>, String> {
    let item = batch
        .evidence
        .get(index)
        .ok_or("LEARN_SYNC_EVIDENCE_INVALID")?;
    let selected = batch
        .runs
        .iter()
        .find(|r| r.run_id == item.run_id)
        .ok_or("LEARN_SYNC_EVIDENCE_INVALID")?;
    let raw: String = db
        .query_row(
            "SELECT record FROM learn_project_runs WHERE user_id=?1 AND run_id=?2",
            params![owner, item.run_id],
            |r| r.get(0),
        )
        .map_err(|_| "LEARN_RUN_NOT_FOUND")?;
    let saved: Stored = serde_json::from_str(&raw).map_err(|_| "LEARN_RUN_INVALID")?;
    let preview = evidence::restore(db, owner, &saved)?.ok_or("LEARN_SYNC_PREVIEW_REQUIRED")?;
    let bytes = preview
        .items
        .iter()
        .find(|p| p.kind == item.kind)
        .ok_or("LEARN_SYNC_EVIDENCE_INVALID")?;
    if preview.snapshot_sha256 != selected.snapshot_sha256
        || bytes.sha256 != item.sha256
        || bytes.size != item.size
        || bytes.content_type != item.content_type
    {
        return Err("LEARN_EVIDENCE_CHANGED".into());
    }
    Ok(bytes.text.as_bytes().to_vec())
}

#[cfg(test)]
pub(super) mod tests;

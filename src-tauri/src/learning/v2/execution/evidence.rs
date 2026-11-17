//! Complete, immutable local previews. No upload or assessment authority.
use super::*;
use rusqlite::OptionalExtension;

const LIMIT: usize = 4 * 1024 * 1024;
const CHUNK: usize = 49152;
const INVALID: &str = "LEARN_EVIDENCE_INVALID";
const KINDS: [&str; 3] = ["stdout", "build_log", "exit_status"];

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceItem {
    pub kind: String,
    pub state: String,
    pub started_at: String,
    pub finished_at: String,
    pub content_type: String,
    pub sha256: String,
    pub size: usize,
    pub raw_sha256: String,
    pub raw_bytes: usize,
    pub redacted: bool,
    pub output_truncated: bool,
    pub collector_version: String,
    pub text: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chunk {
    run_id: String,
    kind: String,
    state: String,
    started_at: String,
    finished_at: String,
    content_type: String,
    sha256: String,
    size: usize,
    raw_sha256: String,
    raw_bytes: usize,
    redacted: bool,
    output_truncated: bool,
    collector_version: String,
    offset: usize,
    next_offset: usize,
    data_base64: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidencePreview {
    pub snapshot_sha256: String,
    pub run_id: String,
    pub request: ActionRequest,
    pub binding_digest: String,
    pub collected_at: String,
    pub trust: String,
    pub sync_status: String,
    pub items: Vec<EvidenceItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_context: Option<super::provenance::ExecutionContext>,
}
pub(super) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_action_evidence_previews(user_id INTEGER NOT NULL,run_id TEXT NOT NULL,scope TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(user_id,run_id));")
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn time_valid(value: &str) -> bool {
    value.len() <= 64 && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}
fn validate_item(saved: &Stored, item: &EvidenceItem, complete: bool) -> Result<(), String> {
    let result = &saved.run.result;
    if !KINDS.contains(&item.kind.as_str())
        || ![
            "succeeded",
            "failed",
            "cancelled",
            "timed_out",
            "output_limit",
        ]
        .contains(&item.state.as_str())
        || item.state != saved.run.state
        || result["exitCode"].as_i64().is_none()
        || result["startedAt"].as_str() != Some(&item.started_at)
        || result["finishedAt"].as_str() != Some(&item.finished_at)
        || !time_valid(&item.started_at)
        || !time_valid(&item.finished_at)
        || chrono::DateTime::parse_from_rfc3339(&item.finished_at).ok()
            < chrono::DateTime::parse_from_rfc3339(&item.started_at).ok()
        || item.collector_version != "lynx-project-evidence/1"
        || !digest_valid(&item.sha256)
        || !digest_valid(&item.raw_sha256)
        || item.size > LIMIT
        || item.raw_bytes > LIMIT / 4
        || item.output_truncated != (item.state == "output_limit")
        || item.redacted != (item.sha256 != item.raw_sha256)
        || (!item.redacted && item.raw_bytes != item.size)
        || item.content_type
            != if item.kind == "exit_status" {
                "application/json"
            } else {
                "text/plain"
            }
        || (item.kind == "build_log"
            && (result["logSha256"].as_str() != Some(&item.raw_sha256)
                || result["logBytes"].as_u64() != Some(item.raw_bytes as u64)))
    {
        return Err(INVALID.into());
    }
    if complete {
        if item.text.len() != item.size || sha(item.text.as_bytes()) != item.sha256 {
            return Err(INVALID.into());
        }
        if item.kind == "exit_status"
            && (item.redacted
                || serde_json::from_str::<i64>(&item.text).ok() != result["exitCode"].as_i64())
        {
            return Err(INVALID.into());
        }
    }
    Ok(())
}
struct Assembler {
    kind: String,
    metadata: Option<EvidenceItem>,
    bytes: Vec<u8>,
    done: bool,
}
impl Assembler {
    fn new(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            metadata: None,
            bytes: vec![],
            done: false,
        }
    }
    fn request(&self, saved: &Stored) -> Value {
        json!({"operation":"evidence","runId":saved.run.run_id,"scope":saved.scope,"kind":self.kind,
            "offset":self.bytes.len(),"length":CHUNK,
            "expectedSha256":self.metadata.as_ref().map_or("",|m| m.sha256.as_str()),
            "expectedRawSha256":self.metadata.as_ref().map_or("",|m| m.raw_sha256.as_str())})
    }
    fn push(&mut self, saved: &Stored, value: Value) -> Result<(), String> {
        let c: Chunk = serde_json::from_value(value).map_err(|_| INVALID)?;
        if self.done
            || c.run_id != saved.run.run_id
            || c.kind != self.kind
            || c.offset != self.bytes.len()
            || c.data_base64.len() > CHUNK * 4 / 3
        {
            return Err(INVALID.into());
        }
        let m = EvidenceItem {
            kind: c.kind,
            state: c.state,
            started_at: c.started_at,
            finished_at: c.finished_at,
            content_type: c.content_type,
            sha256: c.sha256,
            size: c.size,
            raw_sha256: c.raw_sha256,
            raw_bytes: c.raw_bytes,
            redacted: c.redacted,
            output_truncated: c.output_truncated,
            collector_version: c.collector_version,
            text: String::new(),
        };
        validate_item(saved, &m, false)?;
        if self.metadata.as_ref().is_some_and(|old| old != &m) {
            return Err("LEARN_EVIDENCE_CHANGED".into());
        }
        let bytes = STANDARD.decode(c.data_base64).map_err(|_| INVALID)?;
        if c.offset > m.size
            || bytes.len() != CHUNK.min(m.size - c.offset)
            || c.next_offset != c.offset + bytes.len()
        {
            return Err(INVALID.into());
        }
        self.bytes.extend(bytes);
        self.done = c.next_offset == m.size;
        self.metadata = Some(m);
        Ok(())
    }
    fn finish(self, saved: &Stored) -> Result<EvidenceItem, String> {
        if !self.done {
            return Err(INVALID.into());
        }
        let mut item = self.metadata.ok_or(INVALID)?;
        item.text = String::from_utf8(self.bytes).map_err(|_| INVALID)?;
        validate_item(saved, &item, true)?;
        Ok(item)
    }
}
fn preview_digest(preview: &EvidencePreview) -> Result<String, String> {
    let mut unsigned = preview.clone();
    unsigned.snapshot_sha256.clear();
    Ok(sha(&serde_json::to_vec(&unsigned).map_err(|_| INVALID)?))
}
fn validate_preview(saved: &Stored, preview: &EvidencePreview) -> Result<(), String> {
    if preview.run_id != saved.run.run_id
        || preview.binding_digest != saved.binding_digest
        || serde_json::to_value(&preview.request).map_err(|_| INVALID)?
            != serde_json::to_value(&saved.run.request).map_err(|_| INVALID)?
        || preview.trust != "client_observation"
        || preview.sync_status != "local_only"
        || !time_valid(&preview.collected_at)
        || preview.snapshot_sha256 != preview_digest(preview)?
        || preview.execution_context != super::provenance::context(saved)?
        || preview.items.len() != KINDS.len()
    {
        return Err(INVALID.into());
    }
    for (item, kind) in preview.items.iter().zip(KINDS) {
        if item.kind != kind {
            return Err(INVALID.into());
        }
        validate_item(saved, item, true)?;
    }
    Ok(())
}
pub(super) fn restore(
    db: &Connection,
    owner: i64,
    saved: &Stored,
) -> Result<Option<EvidencePreview>, String> {
    let row: Option<(String,String)> = db.query_row("SELECT scope,record FROM learn_action_evidence_previews WHERE user_id=?1 AND run_id=?2",params![owner,saved.run.run_id],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let Some((scope, raw)) = row else {
        return Ok(None);
    };
    if scope != saved.scope || raw.len() > 8 * LIMIT {
        return Err(INVALID.into());
    }
    let preview: EvidencePreview = serde_json::from_str(&raw).map_err(|_| INVALID)?;
    validate_preview(saved, &preview)?;
    Ok(Some(preview))
}
fn persist(
    local: &LocalLearningState,
    owner: i64,
    saved: &Stored,
    preview: &EvidencePreview,
) -> Result<(), String> {
    validate_preview(saved, preview)?;
    let mut db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let tx = db.transaction().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let current: String = tx
        .query_row(
            "SELECT record FROM learn_project_runs WHERE user_id=?1 AND run_id=?2",
            params![owner, saved.run.run_id],
            |r| r.get(0),
        )
        .map_err(|_| "LEARN_RUN_NOT_FOUND")?;
    let current: Stored = serde_json::from_str(&current).map_err(|_| INVALID)?;
    if current.revision != saved.revision || current.scope != saved.scope {
        return Err("LEARN_RUN_CONFLICT".into());
    }
    if let Some(existing) = restore(&tx, owner, saved)? {
        if existing.snapshot_sha256 != preview.snapshot_sha256 {
            return Err("LEARN_EVIDENCE_CHANGED".into());
        }
        return Ok(());
    }
    let raw = serde_json::to_string(preview).map_err(|_| INVALID)?;
    if raw.len() > 8 * LIMIT {
        return Err(INVALID.into());
    }
    tx.execute(
        "INSERT INTO learn_action_evidence_previews VALUES(?1,?2,?3,?4)",
        params![owner, saved.run.run_id, saved.scope, raw],
    )
    .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    tx.commit().map_err(|_| "LEARN_STORE_UNAVAILABLE".into())
}
async fn check_owner(auth: &AuthState, owner: i64) -> Result<(), String> {
    if user(auth).await? != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn learn_collect_action_evidence(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    run_id: String,
) -> Result<EvidencePreview, String> {
    let owner = user(&auth).await?;
    let saved = load(&local, owner, &run_id)?;
    // A complete cached preview needs no SSH or course fetch. It remains local history.
    let restored = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        restore(&db, owner, &saved)?
    };
    if let Some(preview) = restored {
        check_owner(&auth, owner).await?;
        return Ok(preview);
    }
    let (checked, course) = environment::verified(&auth, &saved.run.request.environment).await?;
    if checked != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    task(&course, &saved.run.request)?;
    if ![
        "succeeded",
        "failed",
        "cancelled",
        "timed_out",
        "output_limit",
    ]
    .contains(&saved.run.state.as_str())
        || saved.run.result["exitCode"].as_i64().is_none()
        || saved.run.result["startedAt"].as_str().is_none()
        || saved.run.result["finishedAt"].as_str().is_none()
    {
        return Err("LEARN_EVIDENCE_NOT_AVAILABLE".into());
    }
    let check_binding = || -> Result<(), String> {
        let (binding, _, _) =
            environment::bound(&local, &workbench, owner, &saved.run.request.environment)?;
        if binding.config_digest != saved.binding_digest {
            return Err("LEARN_BINDING_CHANGED".into());
        }
        Ok(())
    };
    check_binding()?;
    let mut items = Vec::new();
    for kind in KINDS {
        let mut assembler = Assembler::new(kind);
        while !assembler.done {
            check_owner(&auth, owner).await?;
            check_binding()?;
            let response = remote(saved.host.clone(), assembler.request(&saved)).await?;
            assembler.push(&saved, response)?;
        }
        items.push(assembler.finish(&saved)?);
    }
    let mut preview = EvidencePreview {
        snapshot_sha256: String::new(),
        run_id: saved.run.run_id.clone(),
        request: saved.run.request.clone(),
        binding_digest: saved.binding_digest.clone(),
        collected_at: chrono::Utc::now().to_rfc3339(),
        trust: "client_observation".into(),
        sync_status: "local_only".into(),
        items,
        execution_context: super::provenance::context(&saved)?,
    };
    preview.snapshot_sha256 = preview_digest(&preview)?;
    check_owner(&auth, owner).await?;
    check_binding()?;
    persist(&local, owner, &saved, &preview)?;
    check_owner(&auth, owner).await?;
    Ok(preview)
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;

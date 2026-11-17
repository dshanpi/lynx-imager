//! Signed, execution-time local artifacts. Reads never fetch current remote files.
use super::*;

pub(super) const CHUNK: usize = 65536;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;
// Leave room for metadata hashes and the bounded diagnostic in the 32 KiB evidence envelope.
const MAX_SPECS_JSON: usize = 8192;
const INVALID: &str = "LEARNING_ARTIFACT_INVALID:产物快照校验失败，请保留记录并重新核对";
const MISSING: &str = "LEARNING_ARTIFACT_NOT_FOUND:此执行记录没有所选产物快照";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSpec {
    pub(super) artifact_id: String,
    pub(super) title: String,
    pub(super) path: String,
    pub(super) mime_type: String,
    pub(super) max_bytes: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactMeta {
    pub spec: ArtifactSpec,
    pub bytes: u64,
    pub sha256: String,
    pub chunk_root: String,
    pub chunk_count: usize,
}
pub(super) struct Collected {
    pub meta: ArtifactMeta,
    pub data: Vec<u8>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactScope {
    pub session_id: String,
    pub step_id: String,
    pub attempt: i64,
    pub digest: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkRequest {
    pub scope: ArtifactScope,
    pub artifact_id: String,
    pub sha256: String,
    pub offset: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactList {
    pub scope: ArtifactScope,
    pub created_at: i64,
    pub items: Vec<ArtifactMeta>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChunk {
    pub scope: ArtifactScope,
    pub artifact_id: String,
    pub sha256: String,
    pub offset: u64,
    pub next_offset: u64,
    pub bytes: u64,
    pub data_base64: String,
}

pub(super) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learning_artifact_chunks(evidence_id INTEGER NOT NULL REFERENCES learning_evidence(id),artifact_id TEXT NOT NULL,chunk_index INTEGER NOT NULL,sha256 TEXT NOT NULL,body BLOB NOT NULL,PRIMARY KEY(evidence_id,artifact_id,chunk_index));")
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_spec(spec: &ArtifactSpec) -> bool {
    valid_id(&spec.artifact_id)
        && !spec.title.trim().is_empty()
        && spec.title.chars().count() <= 120
        && !spec.title.chars().any(char::is_control)
        && safe_relative(&spec.path).is_ok()
        && spec.path.len() <= 2048
        && (1..=MAX_FILE).contains(&spec.max_bytes)
        && matches!(
            spec.mime_type.as_str(),
            "application/json"
                | "text/plain"
                | "audio/wav"
                | "audio/flac"
                | "video/mp4"
                | "application/octet-stream"
        )
}
pub(super) fn validate(manifest: &LocalCourseManifest, lab: &LocalLab) -> Result<(), String> {
    let specs = &lab.artifact_outputs;
    if specs.is_empty() {
        return Ok(());
    }
    let mut ids = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    if !matches!(manifest.schema_version, 5..=8)
        || theory::is_reflection(lab)
        || lab.hil_verification.is_some()
        || specs.len() > 8
        || specs
            .iter()
            .any(|s| !valid_spec(s) || !ids.insert(&s.artifact_id) || !paths.insert(&s.path))
        || specs.iter().map(|s| s.max_bytes).sum::<u64>() > MAX_TOTAL
        || serde_json::to_vec(specs).map_err(|_| INVALID)?.len() > MAX_SPECS_JSON
    {
        return Err("LOCAL_COURSE_ARTIFACT_INVALID:产物声明格式、类型或读取预算无效".into());
    }
    Ok(())
}
fn body_valid(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "application/json" => serde_json::from_slice::<serde_json::Value>(bytes).is_ok(),
        "text/plain" => std::str::from_utf8(bytes).is_ok(),
        "audio/wav" => {
            bytes.len() >= 12
                && &bytes[..4] == b"RIFF"
                && &bytes[8..12] == b"WAVE"
                && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8 == bytes.len()
        }
        "audio/flac" => bytes.len() >= 42 && &bytes[..4] == b"fLaC",
        "video/mp4" => {
            bytes.len() >= 12
                && &bytes[4..8] == b"ftyp"
                && (12..=bytes.len())
                    .contains(&(u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize))
        }
        "application/octet-stream" => true,
        _ => false,
    }
}
fn chunk_root(hashes: &[String]) -> String {
    sha256(hashes.concat().as_bytes())
}
pub(super) fn collected(spec: ArtifactSpec, data: Vec<u8>) -> Result<Collected, String> {
    if !valid_spec(&spec)
        || data.len() as u64 > spec.max_bytes
        || !body_valid(&spec.mime_type, &data)
    {
        return Err(INVALID.into());
    }
    let hashes: Vec<_> = data.chunks(CHUNK).map(sha256).collect();
    let meta = ArtifactMeta {
        spec,
        bytes: data.len() as u64,
        sha256: sha256(&data),
        chunk_root: chunk_root(&hashes),
        chunk_count: hashes.len(),
    };
    Ok(Collected { meta, data })
}
pub(super) fn capture(
    ssh: &ssh2::Session,
    tree: &freshness::Tree,
    specs: &[ArtifactSpec],
) -> Result<Vec<Collected>, String> {
    let mut result = Vec::new();
    let mut total = 0;
    for spec in specs {
        if result.len() >= 8 || !valid_spec(spec) {
            return Err(INVALID.into());
        }
        let data = freshness::artifact_bytes(ssh, tree, &spec.path, spec.max_bytes)?;
        total += data.len() as u64;
        if total > MAX_TOTAL {
            return Err(INVALID.into());
        }
        result.push(collected(spec.clone(), data)?);
    }
    Ok(result)
}
pub(super) fn store(db: &Connection, evidence_id: i64, items: &[Collected]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    if items.len() > 8
        || items.iter().any(|item| {
            item.data.len() as u64 > MAX_FILE || !ids.insert(&item.meta.spec.artifact_id)
        })
        || items.iter().map(|item| item.data.len() as u64).sum::<u64>() > MAX_TOTAL
        || serde_json::to_vec(&items.iter().map(|item| &item.meta.spec).collect::<Vec<_>>())
            .map_err(|_| INVALID)?
            .len()
            > MAX_SPECS_JSON
    {
        return Err(INVALID.into());
    }
    for item in items {
        let verified = collected(item.meta.spec.clone(), item.data.clone())?;
        if serde_json::to_value(&verified.meta).map_err(|_| INVALID)?
            != serde_json::to_value(&item.meta).map_err(|_| INVALID)?
        {
            return Err(INVALID.into());
        }
        for (index, bytes) in item.data.chunks(CHUNK).enumerate() {
            db.execute("INSERT INTO learning_artifact_chunks(evidence_id,artifact_id,chunk_index,sha256,body) VALUES(?1,?2,?3,?4,?5)",
                params![evidence_id,item.meta.spec.artifact_id,index as i64,sha256(bytes),bytes]).map_err(|_| INVALID)?;
        }
    }
    Ok(())
}
fn metadata(db: &Connection, scope: &ArtifactScope) -> Result<(i64, ArtifactList), String> {
    if scope.session_id.is_empty()
        || scope.session_id.len() > 256
        || !valid_id(&scope.step_id)
        || scope.attempt <= 0
        || !valid_digest(&scope.digest)
    {
        return Err(INVALID.into());
    }
    let (id, raw, created_at):(i64,Option<String>,i64)=db.query_row("SELECT id,CASE WHEN length(CAST(detail_json AS BLOB))<=32768 THEN detail_json ELSE NULL END,created_at FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND digest=?4 AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1",params![scope.session_id,scope.step_id,scope.attempt,scope.digest],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|MISSING)?;
    let raw = raw.ok_or(INVALID)?;
    if raw.len() > 32768 || sha256(raw.as_bytes()) != scope.digest {
        return Err(INVALID.into());
    }
    let detail: serde_json::Value = serde_json::from_str(&raw).map_err(|_| INVALID)?;
    let items: Vec<ArtifactMeta> = match detail.get("artifactVersion") {
        None => Vec::new(),
        Some(v) if v.as_u64() == Some(1) => {
            serde_json::from_value(detail.get("artifacts").cloned().ok_or(INVALID)?)
                .map_err(|_| INVALID)?
        }
        _ => return Err(INVALID.into()),
    };
    let mut ids = std::collections::HashSet::new();
    if items.len() > 8
        || items.iter().any(|m| {
            !valid_spec(&m.spec)
                || m.bytes > m.spec.max_bytes
                || !valid_digest(&m.sha256)
                || !valid_digest(&m.chunk_root)
                || m.chunk_count != (m.bytes as usize).div_ceil(CHUNK)
                || !ids.insert(&m.spec.artifact_id)
        })
        || items.iter().map(|m| m.bytes).sum::<u64>() > MAX_TOTAL
    {
        return Err(INVALID.into());
    }
    Ok((
        id,
        ArtifactList {
            scope: scope.clone(),
            created_at,
            items,
        },
    ))
}
fn index(db: &Connection, id: i64, meta: &ArtifactMeta) -> Result<Vec<String>, String> {
    let mut statement=db.prepare("SELECT chunk_index,sha256,length(body) FROM learning_artifact_chunks WHERE evidence_id=?1 AND artifact_id=?2 ORDER BY chunk_index").map_err(|_|INVALID)?;
    let rows = statement
        .query_map(params![id, meta.spec.artifact_id], |r| {
            Ok((
                r.get::<_, usize>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
            ))
        })
        .map_err(|_| INVALID)?;
    let mut hashes = Vec::new();
    for row in rows {
        let (offset, hash, size) = row.map_err(|_| INVALID)?;
        if hashes.len() >= meta.chunk_count
            || offset != hashes.len()
            || !valid_digest(&hash)
            || size != (meta.bytes - offset as u64 * CHUNK as u64).min(CHUNK as u64)
        {
            return Err(INVALID.into());
        }
        hashes.push(hash);
    }
    if hashes.len() != meta.chunk_count || chunk_root(&hashes) != meta.chunk_root {
        return Err(INVALID.into());
    }
    Ok(hashes)
}
pub(super) fn list(db: &Connection, scope: &ArtifactScope) -> Result<ArtifactList, String> {
    let (id, list) = metadata(db, scope)?;
    for meta in &list.items {
        index(db, id, meta)?;
    }
    Ok(list)
}
pub(super) fn read_chunk(db: &Connection, request: &ChunkRequest) -> Result<ArtifactChunk, String> {
    let (id, list) = metadata(db, &request.scope)?;
    let meta = list
        .items
        .iter()
        .find(|m| m.spec.artifact_id == request.artifact_id)
        .ok_or(MISSING)?;
    if request.sha256 != meta.sha256
        || !request.offset.is_multiple_of(CHUNK as u64)
        || (request.offset >= meta.bytes && !(request.offset == 0 && meta.bytes == 0))
    {
        return Err(INVALID.into());
    }
    let hashes = index(db, id, meta)?;
    let data: Vec<u8> = if meta.bytes == 0 {
        Vec::new()
    } else {
        let chunk_index = request.offset as usize / CHUNK;
        let bytes:Vec<u8>=db.query_row("SELECT body FROM learning_artifact_chunks WHERE evidence_id=?1 AND artifact_id=?2 AND chunk_index=?3",params![id,request.artifact_id,chunk_index as i64],|r|r.get(0)).map_err(|_|INVALID)?;
        if bytes.len() as u64 != (meta.bytes - request.offset).min(CHUNK as u64)
            || sha256(&bytes) != hashes[chunk_index]
        {
            return Err(INVALID.into());
        }
        bytes
    };
    Ok(ArtifactChunk {
        scope: request.scope.clone(),
        artifact_id: request.artifact_id.clone(),
        sha256: meta.sha256.clone(),
        offset: request.offset,
        next_offset: request.offset + data.len() as u64,
        bytes: meta.bytes,
        data_base64: STANDARD.encode(&data),
    })
}
#[command]
pub fn learning_list_local_artifacts(
    local: State<'_, Arc<LocalLearningState>>,
    request: ArtifactScope,
) -> Result<ArtifactList, String> {
    let db = local.db.lock().map_err(|_| INVALID)?;
    list(&db, &request)
}
#[command]
pub fn learning_read_local_artifact_chunk(
    local: State<'_, Arc<LocalLearningState>>,
    request: ChunkRequest,
) -> Result<ArtifactChunk, String> {
    let db = local.db.lock().map_err(|_| INVALID)?;
    read_chunk(&db, &request)
}

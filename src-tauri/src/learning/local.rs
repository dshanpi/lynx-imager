use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{command, State};

use crate::{supervisor::SupervisorState, workbench::WorkbenchState};

pub mod artifacts;
pub mod diagnostics;
pub mod environment;
mod environment_receipt;
#[cfg(test)]
mod environment_receipt_tests;
mod environment_report;
#[cfg(test)]
mod environment_tests;
mod execution;
mod freshness;
mod hil_trust;
#[cfg(test)]
mod hil_trust_tests;
pub mod journey;
pub mod optional;
#[cfg(test)]
mod optional_tests;
mod prerequisites;
#[cfg(test)]
mod prerequisites_tests;
pub mod report;
#[cfg(test)]
mod report_tests;
pub mod revisit;
#[cfg(test)]
mod revisit_tests;
pub mod theory;
mod tutor_context;

#[cfg(test)]
mod review_tests;

#[cfg(test)]
mod diagnostics_tests;

#[cfg(test)]
mod tutor_context_tests;

#[cfg(test)]
mod theory_tests;

#[cfg(test)]
mod artifacts_tests;

const MAX_COURSE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 768 * 1024 * 1024;
const VALID_STATES: &[&str] = &[
    "reading",
    "checking_understanding",
    "practicing",
    "collecting_evidence",
    "verifying",
    "reflecting",
    "completed",
];

pub struct LocalLearningState {
    pub db: Mutex<Connection>,
    root: PathBuf,
    actions: execution::ActionGate,
}

impl LocalLearningState {
    pub fn open(path: &Path, root: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        migrate(&db).map_err(|e| e.to_string())?;
        crate::learning::v2::environment::migrate(&db).map_err(|e| e.to_string())?;
        crate::learning::v2::execution::migrate(&db).map_err(|e| e.to_string())?;
        crate::learning::v2::board::migrate(&db).map_err(|e| e.to_string())?;
        crate::learning::v2::preview::discovery::migrate(&db).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Mutex::new(db),
            root,
            actions: execution::ActionGate::default(),
        })
    }
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS local_course_signers(fingerprint TEXT PRIMARY KEY,public_key TEXT NOT NULL,trusted_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS local_courses(course_id TEXT NOT NULL,version TEXT NOT NULL,title TEXT NOT NULL,board_profile TEXT NOT NULL,signer_fingerprint TEXT NOT NULL,package_sha256 TEXT NOT NULL,manifest_json TEXT NOT NULL,install_path TEXT NOT NULL,installed_at INTEGER NOT NULL,PRIMARY KEY(course_id,version));
         CREATE TABLE IF NOT EXISTS learning_sessions(session_id TEXT PRIMARY KEY,course_id TEXT NOT NULL,course_version TEXT NOT NULL,project_id INTEGER NOT NULL,current_step TEXT NOT NULL,state TEXT NOT NULL,classification TEXT NOT NULL DEFAULT 'INCOMPLETE',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS learning_attempts(id INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,step_id TEXT NOT NULL,attempt INTEGER NOT NULL,state TEXT NOT NULL,answer TEXT NOT NULL DEFAULT '',reflection TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(session_id,step_id,attempt));
         CREATE TABLE IF NOT EXISTS learning_evidence(id INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,step_id TEXT NOT NULL,attempt INTEGER NOT NULL,state TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,source TEXT NOT NULL,status TEXT NOT NULL,summary TEXT NOT NULL,digest TEXT NOT NULL,detail_json TEXT NOT NULL,created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS learning_hil_approvals(token_hash TEXT PRIMARY KEY,session_id TEXT NOT NULL,step_id TEXT NOT NULL,attempt INTEGER NOT NULL DEFAULT 1,action_id TEXT NOT NULL,device_id INTEGER NOT NULL,board_uid TEXT NOT NULL,target_json TEXT NOT NULL,expires_at INTEGER NOT NULL,used_at INTEGER);"
    )?;
    add_column(db, "learning_sessions", "device_id INTEGER")?;
    add_column(
        db,
        "learning_sessions",
        "board_uid TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(
        db,
        "learning_sessions",
        "course_package_sha256 TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(db, "learning_evidence", "state TEXT NOT NULL DEFAULT ''")?;
    add_column(
        db,
        "learning_hil_approvals",
        "attempt INTEGER NOT NULL DEFAULT 1",
    )?;
    journey::migrate_journey(db)?;
    artifacts::migrate(db)?;
    Ok(())
}

fn add_column(db: &Connection, table: &str, definition: &str) -> rusqlite::Result<()> {
    let column = definition.split_whitespace().next().unwrap_or_default();
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        db.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), [])?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CourseEnvelope {
    format: String,
    algorithm: String,
    public_key: String,
    sha256: String,
    signature: String,
    payload: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCourseManifest {
    schema_version: u32,
    course_id: String,
    version: String,
    title: String,
    board_profile: String,
    provenance_sha256: String,
    labs: Vec<LocalLab>,
    #[serde(default)]
    workspace: Option<LearnWorkspaceSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnWorkspaceSpec {
    remote_subdir: String,
    package_source_root: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnDocumentSpec {
    path: String,
    sha256: String,
    source_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prerequisites: Option<Vec<String>>,
    #[serde(default)]
    environment_checks: Vec<environment::CheckSpec>,
    #[serde(default = "optional::required_by_default")]
    required: bool,
    #[serde(default)]
    artifact_outputs: Vec<artifacts::ArtifactSpec>,
    #[serde(default = "theory::default_activity_kind")]
    activity_kind: String,
    step_id: String,
    number: u32,
    path: String,
    title: String,
    learning_objective: String,
    comprehension_check: String,
    student_action: String,
    risk_level: String,
    expected_observation: String,
    verification_rules: Vec<VerificationRule>,
    required_evidence: Vec<EvidenceRequirement>,
    reflection_prompt: String,
    #[serde(default)]
    hil_verification: Option<HilVerification>,
    #[serde(default)]
    document: Option<LearnDocumentSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HilVerification {
    action_id: String,
    target: String,
    instructions: String,
    required_evidence_kinds: Vec<String>,
    expected_observations: Vec<String>,
    #[serde(default)]
    minimum_duration_seconds: Option<u64>,
    #[serde(default)]
    image: Option<HilImage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HilImage {
    path: String,
    bytes: u64,
    md5: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationRule {
    id: String,
    kind: String,
    command: String,
    expected_exit_code: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EvidenceRequirement {
    kind: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProvenanceLock {
    schema_version: u32,
    source_files: Vec<LockedFile>,
}

#[derive(Deserialize)]
struct LockedFile {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnChapterSummary {
    step_id: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLocalCourse {
    course_id: String,
    version: String,
    title: String,
    board_profile: String,
    signer_fingerprint: String,
    package_sha256: String,
    lab_count: usize,
    entry_requires_project: bool,
    chapters: Vec<LearnChapterSummary>,
    installed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLearningSession {
    session_id: String,
    course_id: String,
    course_version: String,
    project_id: i64,
    device_id: Option<i64>,
    board_uid: String,
    course_package_sha256: String,
    current_step: String,
    state: String,
    classification: String,
    recorded_classification: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLearningStepView {
    session: LocalLearningSession,
    lab: LocalLab,
    step_index: usize,
    total_steps: usize,
    evidence_count: usize,
    evidence: Vec<LearningEvidenceView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningEvidenceView {
    recorded: Option<hil_trust::RecordedEvidence>,
    kind: String,
    source: String,
    status: String,
    summary: String,
    digest: String,
    created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTutorRequest {
    session_id: String,
    question: String,
    hint_level: String,
    #[serde(default)]
    expected_step_id: Option<String>,
    #[serde(default)]
    expected_attempt: Option<i64>,
    #[serde(default)]
    diagnostic_digest: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTutorAnswer {
    answer: String,
    context_scope: Vec<String>,
    executed_action: bool,
    fixture: bool,
}

#[derive(Serialize)]
struct TutorBody {
    model: String,
    messages: Vec<TutorMessage>,
    max_tokens: u32,
    stream: bool,
}
#[derive(Serialize)]
struct TutorMessage {
    role: &'static str,
    content: String,
}
#[derive(Deserialize)]
struct TutorResponse {
    choices: Vec<TutorChoice>,
}
#[derive(Deserialize)]
struct TutorChoice {
    message: TutorResponseMessage,
}
#[derive(Deserialize)]
struct TutorResponseMessage {
    content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepActionResult {
    status: String,
    source: String,
    exit_code: Option<i32>,
    output_digest: String,
    summary: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HilPreparation {
    action_id: String,
    target: String,
    instructions: String,
    device_id: i64,
    board_uid: String,
    confirmation_phrase: String,
    approval_token: String,
    expires_at: i64,
    required_evidence_kinds: Vec<String>,
    expected_observations: Vec<String>,
    minimum_duration_seconds: Option<u64>,
    image: Option<HilImage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HilConfirmationRequest {
    session_id: String,
    approval_token: String,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HilEvidenceRequest {
    #[serde(default)]
    expected_step_id: Option<String>,
    #[serde(default)]
    expected_attempt: Option<i64>,
    session_id: String,
    evidence_kind: String,
    status: String,
    summary: String,
    observation: String,
    #[serde(default)]
    artifact_sha256: Option<String>,
    #[serde(default)]
    duration_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HilApprovalResult {
    status: String,
    action_id: String,
    board_uid: String,
    evidence_digest: String,
    instructions: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningReport {
    prerequisite_checks: Vec<prerequisites::Access>,
    required_steps: usize,
    required_completed_steps: usize,
    optional_steps: usize,
    optional_completed_steps: usize,
    optional_skipped_steps: usize,
    session: LocalLearningSession,
    completed_steps: usize,
    total_steps: usize,
    evidence_count: usize,
    software_result: String,
    hil_result: String,
    recorded_hil_result: String,
    hil_passed_steps: usize,
    hil_required_steps: usize,
    provenance_result: String,
    recorded_software_result: String,
    current_workspace: report::WorkspaceCurrency,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs() as i64)
        .unwrap_or_default()
}
fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn safe_relative(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains('\\') || value.contains('\0') {
        return Err("LOCAL_COURSE_UNSAFE_PATH".into());
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("LOCAL_COURSE_UNSAFE_PATH".into());
    }
    Ok(path.to_path_buf())
}

fn verify_envelope(data: &[u8]) -> Result<(CourseEnvelope, Vec<u8>, String), String> {
    let envelope: CourseEnvelope =
        serde_json::from_slice(data).map_err(|_| "LOCAL_COURSE_ENVELOPE_INVALID")?;
    if envelope.format != "lynx-course-v1" || envelope.algorithm != "Ed25519-SHA256" {
        return Err("LOCAL_COURSE_FORMAT_UNSUPPORTED".into());
    }
    let payload = STANDARD
        .decode(&envelope.payload)
        .map_err(|_| "LOCAL_COURSE_PAYLOAD_INVALID")?;
    let digest = Sha256::digest(&payload);
    if hex::encode(digest) != envelope.sha256.to_ascii_lowercase() {
        return Err("LOCAL_COURSE_HASH_MISMATCH".into());
    }
    let key_bytes: [u8; 32] = STANDARD
        .decode(&envelope.public_key)
        .map_err(|_| "LOCAL_COURSE_SIGNER_INVALID")?
        .try_into()
        .map_err(|_| "LOCAL_COURSE_SIGNER_INVALID")?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| "LOCAL_COURSE_SIGNER_INVALID")?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(&envelope.signature)
            .map_err(|_| "LOCAL_COURSE_SIGNATURE_INVALID")?,
    )
    .map_err(|_| "LOCAL_COURSE_SIGNATURE_INVALID")?;
    key.verify(&digest, &signature)
        .map_err(|_| "LOCAL_COURSE_SIGNATURE_INVALID")?;
    let fingerprint = sha256(&key_bytes);
    Ok((envelope, payload, fingerprint))
}

fn inspect_zip(payload: &[u8]) -> Result<(LocalCourseManifest, ProvenanceLock), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(payload))
        .map_err(|_| "LOCAL_COURSE_ARCHIVE_INVALID")?;
    if zip.is_empty() || zip.len() > 8192 {
        return Err("LOCAL_COURSE_ARCHIVE_INVALID".into());
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|_| "LOCAL_COURSE_ARCHIVE_INVALID")?;
        safe_relative(entry.name())?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err("LOCAL_COURSE_SYMLINK_FORBIDDEN".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("LOCAL_COURSE_TOO_LARGE")?;
        if total > MAX_UNPACKED_BYTES {
            return Err("LOCAL_COURSE_TOO_LARGE".into());
        }
    }
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .map_err(|_| "LOCAL_COURSE_MANIFEST_MISSING")?
        .read_to_end(&mut manifest_bytes)
        .map_err(|_| "LOCAL_COURSE_MANIFEST_INVALID")?;
    let manifest: LocalCourseManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "LOCAL_COURSE_MANIFEST_INVALID")?;
    let mut provenance_bytes = Vec::new();
    zip.by_name("provenance/provenance.lock.json")
        .map_err(|_| "LOCAL_COURSE_PROVENANCE_MISSING")?
        .read_to_end(&mut provenance_bytes)
        .map_err(|_| "LOCAL_COURSE_PROVENANCE_INVALID")?;
    if sha256(&provenance_bytes) != manifest.provenance_sha256 {
        return Err("LOCAL_COURSE_PROVENANCE_MISMATCH".into());
    }
    let provenance: ProvenanceLock =
        serde_json::from_slice(&provenance_bytes).map_err(|_| "LOCAL_COURSE_PROVENANCE_INVALID")?;
    validate_manifest(&manifest)?;
    Ok((manifest, provenance))
}

fn validate_manifest(manifest: &LocalCourseManifest) -> Result<(), String> {
    if !matches!(manifest.schema_version, 1..=8)
        || !valid_id(&manifest.course_id)
        || !valid_id(&manifest.version)
        || (manifest.schema_version < 3 && manifest.labs.len() != 24)
        || manifest.labs.is_empty()
        || manifest.labs.len() > 512
    {
        return Err("LOCAL_COURSE_MANIFEST_INVALID".into());
    }
    if manifest.schema_version < 3 && manifest.workspace.is_some() {
        return Err("LOCAL_COURSE_WORKSPACE_SCHEMA_REQUIRED".into());
    }
    if manifest.schema_version >= 3 && manifest.labs.iter().any(|lab| !theory::is_reflection(lab)) {
        let workspace = manifest
            .workspace
            .as_ref()
            .ok_or("LOCAL_COURSE_WORKSPACE_REQUIRED")?;
        safe_relative(&workspace.remote_subdir)?;
        safe_relative(&workspace.package_source_root)?;
    }
    if let Some(workspace) = &manifest.workspace {
        safe_relative(&workspace.remote_subdir)?;
        safe_relative(&workspace.package_source_root)?;
    }
    prerequisites::validate(manifest)?;
    let mut step_ids = std::collections::HashSet::new();
    for (index, lab) in manifest.labs.iter().enumerate() {
        if !step_ids.insert(&lab.step_id) {
            return Err("LOCAL_COURSE_DUPLICATE_STEP".into());
        }
        if let Some(document) = &lab.document {
            safe_relative(&document.path)?;
            let url =
                url::Url::parse(&document.source_url).map_err(|_| "LOCAL_COURSE_DOCUMENT_URL")?;
            if document.sha256.len() != 64
                || !document.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("LOCAL_COURSE_DOCUMENT_INVALID".into());
            }
        }
        if lab.number as usize != index + 1 || !valid_id(&lab.step_id) {
            return Err("LOCAL_COURSE_LABS_INVALID".into());
        }
        if manifest.schema_version < 6 && !lab.required {
            return Err("LOCAL_COURSE_OPTIONAL_SCHEMA_REQUIRED".into());
        }
        environment::validate(manifest, lab)?;
        theory::validate_activity(manifest, lab)?;
        artifacts::validate(manifest, lab)?;
        if let Some(hil) = &lab.hil_verification {
            if manifest.schema_version < 2
                || lab.risk_level != "hardware-confirmation"
                || !valid_id(&hil.action_id)
                || hil.target != manifest.board_profile
                || hil.required_evidence_kinds.is_empty()
                || hil.expected_observations.is_empty()
                || hil
                    .required_evidence_kinds
                    .iter()
                    .any(|kind| !valid_id(kind))
            {
                return Err("LOCAL_COURSE_HIL_INVALID".into());
            }
            if let Some(image) = &hil.image {
                safe_relative(&image.path)?;
                if image.bytes == 0
                    || image.md5.len() != 32
                    || !image.md5.bytes().all(|b| b.is_ascii_hexdigit())
                    || image.sha256.len() != 64
                    || !image.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("LOCAL_COURSE_HIL_IMAGE_INVALID".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_command(command: &str) -> Result<(), String> {
    execution::render_command(command).map(|_| ())
}

fn extract_zip(
    payload: &[u8],
    target: &Path,
    provenance: &ProvenanceLock,
    manifest: &LocalCourseManifest,
) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(payload))
        .map_err(|_| "LOCAL_COURSE_ARCHIVE_INVALID")?;
    fs::create_dir_all(target).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|_| "LOCAL_COURSE_ARCHIVE_INVALID")?;
        let rel = safe_relative(entry.name())?;
        let out = target.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
            }
            let mut file = fs::File::create(&out).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
            std::io::copy(&mut entry, &mut file).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
            file.flush().map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
        }
    }
    if provenance.schema_version != 1 {
        return Err("LOCAL_COURSE_PROVENANCE_INVALID".into());
    }
    for locked in &provenance.source_files {
        let rel = safe_relative(&locked.path)?;
        let file = target
            .join(
                manifest
                    .workspace
                    .as_ref()
                    .map(|w| w.package_source_root.as_str())
                    .unwrap_or("workspace/web-kvm"),
            )
            .join(rel);
        let bytes = fs::read(file).map_err(|_| "LOCAL_COURSE_SOURCE_MISSING")?;
        if sha256(&bytes) != locked.sha256 {
            return Err("LOCAL_COURSE_SOURCE_MISMATCH".into());
        }
    }
    Ok(())
}

#[command]
pub async fn learning_import_local_course(
    path: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<InstalledLocalCourse, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.extension().and_then(|v| v.to_str()) != Some("lynx-course") {
        return Err("LOCAL_COURSE_PATH_INVALID".into());
    }
    let metadata = fs::metadata(&path).map_err(|_| "LOCAL_COURSE_NOT_FOUND")?;
    if metadata.len() == 0 || metadata.len() > MAX_COURSE_BYTES {
        return Err("LOCAL_COURSE_TOO_LARGE".into());
    }
    let package = fs::read(&path).map_err(|_| "LOCAL_COURSE_READ_FAILED")?;
    let package_sha = sha256(&package);
    let (envelope, payload, fingerprint) = verify_envelope(&package)?;
    let (manifest, provenance) = inspect_zip(&payload)?;
    let root = state.root.clone();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        let trusted: Option<String> = db
            .query_row(
                "SELECT public_key FROM local_course_signers WHERE fingerprint=?1",
                [&fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if trusted.as_ref().is_some_and(|v| v != &envelope.public_key) {
            return Err("LOCAL_COURSE_SIGNER_CHANGED".into());
        }
        let existing: Option<String> = db
            .query_row(
                "SELECT signer_fingerprint FROM local_courses WHERE course_id=?1 LIMIT 1",
                [&manifest.course_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if existing.as_ref().is_some_and(|v| v != &fingerprint) {
            return Err("LOCAL_COURSE_SIGNER_CHANGED".into());
        }
        let target = root
            .join("learning/local-courses")
            .join(&manifest.course_id)
            .join(&manifest.version);
        if target.exists() {
            return Err("LOCAL_COURSE_VERSION_EXISTS".into());
        }
        let staging = root.join("learning/local-courses/.staging").join(format!(
            "{}-{}",
            std::process::id(),
            now()
        ));
        fs::create_dir_all(staging.parent().unwrap()).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
        if let Err(error) = extract_zip(&payload, &staging, &provenance, &manifest) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
        }
        fs::rename(&staging, &target).map_err(|_| "LOCAL_COURSE_INSTALL_FAILED")?;
        let timestamp = now();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO local_course_signers VALUES(?1,?2,?3)",
            params![fingerprint, envelope.public_key, timestamp],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO local_courses VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                manifest.course_id,
                manifest.version,
                manifest.title,
                manifest.board_profile,
                fingerprint,
                package_sha,
                serde_json::to_string(&manifest).unwrap(),
                target.to_string_lossy(),
                timestamp
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(InstalledLocalCourse {
            entry_requires_project: theory::entry_requires_project(&manifest),
            course_id: manifest.course_id,
            version: manifest.version,
            title: manifest.title,
            board_profile: manifest.board_profile,
            signer_fingerprint: fingerprint,
            package_sha256: package_sha,
            lab_count: manifest.labs.len(),
            chapters: manifest
                .labs
                .iter()
                .map(|l| LearnChapterSummary {
                    step_id: l.step_id.clone(),
                    title: l.title.clone(),
                })
                .collect(),
            installed_at: timestamp,
        })
    })
    .await
    .map_err(|_| "LOCAL_COURSE_IMPORT_TASK_FAILED".to_owned())?
}

#[command]
pub fn learning_list_local_courses(
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<Vec<InstalledLocalCourse>, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let mut stmt=db.prepare("SELECT course_id,version,title,board_profile,signer_fingerprint,package_sha256,installed_at,manifest_json FROM local_courses ORDER BY installed_at DESC").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let manifest: String = r.get(7)?;
            let entry_requires_project = serde_json::from_str::<LocalCourseManifest>(&manifest)
                .map(|m| theory::entry_requires_project(&m))
                .unwrap_or(true);
            let chapters = serde_json::from_str::<LocalCourseManifest>(&manifest)
                .map(|m| {
                    m.labs
                        .into_iter()
                        .map(|l| LearnChapterSummary {
                            step_id: l.step_id,
                            title: l.title,
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            Ok(InstalledLocalCourse {
                entry_requires_project,
                course_id: r.get(0)?,
                version: r.get(1)?,
                title: r.get(2)?,
                board_profile: r.get(3)?,
                signer_fingerprint: r.get(4)?,
                package_sha256: r.get(5)?,
                lab_count: chapters.len(),
                chapters,
                installed_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[command]
pub fn learning_start_session(
    course_id: String,
    version: String,
    project_id: i64,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<LocalLearningSession, String> {
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let (manifest_json, package_sha256): (String, String) = db
        .query_row(
            "SELECT manifest_json,package_sha256 FROM local_courses WHERE course_id=?1 AND version=?2",
            params![course_id, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "LOCAL_COURSE_NOT_INSTALLED")?;
    let manifest: LocalCourseManifest =
        serde_json::from_str(&manifest_json).map_err(|_| "LOCAL_COURSE_MANIFEST_INVALID")?;
    validate_manifest(&manifest)?;
    if project_id < 0 || (project_id == 0 && theory::entry_requires_project(&manifest)) {
        return Err("LEARNING_PROJECT_REQUIRED".into());
    }
    let (device_id, board_uid) = if project_id == 0 {
        (None, String::new())
    } else {
        let work = workbench
            .db
            .lock()
            .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
        theory::project_identity(&work, &manifest, project_id)?
    };
    let step = prerequisites::entry(&manifest)
        .ok_or("LOCAL_COURSE_LABS_INVALID")?
        .step_id
        .clone();
    let session = LocalLearningSession {
        session_id: format!("learn-{}-{}", now(), rand::random::<u32>()),
        course_id,
        course_version: version,
        project_id,
        device_id,
        board_uid,
        course_package_sha256: package_sha256,
        current_step: step,
        state: "reading".into(),
        classification: "INCOMPLETE".into(),
        recorded_classification: "INCOMPLETE".into(),
        created_at: now(),
        updated_at: now(),
    };
    insert_session(&db, &session)?;
    get_session(&db, &session.session_id)
}

fn insert_session(db: &Connection, session: &LocalLearningSession) -> Result<(), String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,classification,created_at,updated_at,device_id,board_uid,course_package_sha256) VALUES(?1,?2,?3,?4,?5,'reading','INCOMPLETE',?6,?6,?7,?8,?9)",
        params![session.session_id, session.course_id, session.course_version, session.project_id, session.current_step, session.created_at, session.device_id, session.board_uid, session.course_package_sha256],
    ).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES(?1,?2,1,'reading',?3,?3)",params![session.session_id,session.current_step,session.created_at]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

fn get_session(db: &Connection, id: &str) -> Result<LocalLearningSession, String> {
    db.query_row("SELECT session_id,course_id,course_version,project_id,current_step,state,classification,created_at,updated_at,device_id,board_uid,course_package_sha256 FROM learning_sessions WHERE session_id=?1",[id],|r|Ok(hil_trust::project_session(LocalLearningSession{session_id:r.get(0)?,course_id:r.get(1)?,course_version:r.get(2)?,project_id:r.get(3)?,current_step:r.get(4)?,state:r.get(5)?,classification:r.get(6)?,recorded_classification:r.get(6)?,created_at:r.get(7)?,updated_at:r.get(8)?,device_id:r.get(9)?,board_uid:r.get(10)?,course_package_sha256:r.get(11)?}))).map_err(|_|"LEARNING_SESSION_NOT_FOUND".into())
}

#[command]
pub fn learning_list_local_sessions(
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<Vec<LocalLearningSession>, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let mut stmt = db
        .prepare("SELECT session_id,course_id,course_version,project_id,current_step,state,classification,created_at,updated_at,device_id,board_uid,course_package_sha256 FROM learning_sessions ORDER BY updated_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(hil_trust::project_session(LocalLearningSession {
                session_id: r.get(0)?,
                course_id: r.get(1)?,
                course_version: r.get(2)?,
                project_id: r.get(3)?,
                device_id: r.get(9)?,
                board_uid: r.get(10)?,
                course_package_sha256: r.get(11)?,
                current_step: r.get(4)?,
                state: r.get(5)?,
                classification: r.get(6)?,
                recorded_classification: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[command]
pub fn learning_get_local_step(
    session_id: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LocalLearningStepView, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &session_id)?;
    let (manifest, lab) = manifest_and_lab(&db, &session)?;
    let step_index = manifest
        .labs
        .iter()
        .position(|item| item.step_id == session.current_step)
        .ok_or("LEARNING_STEP_NOT_FOUND")?;
    let evidence_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2)",
            params![session_id, session.current_step],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let evidence = list_step_evidence(&db, &session_id, &session.current_step)?;
    Ok(LocalLearningStepView {
        session,
        lab,
        step_index,
        total_steps: manifest.labs.len(),
        evidence_count: evidence_count as usize,
        evidence,
    })
}

fn list_step_evidence(
    db: &Connection,
    session_id: &str,
    step_id: &str,
) -> Result<Vec<LearningEvidenceView>, String> {
    let mut statement = db
        .prepare("SELECT kind,source,status,summary,digest,created_at FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2) ORDER BY id DESC LIMIT 100")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![session_id, step_id], |row| {
            let mut value = LearningEvidenceView {
                recorded: None,
                kind: row.get(0)?,
                source: row.get(1)?,
                status: row.get(2)?,
                summary: row.get(3)?,
                digest: row.get(4)?,
                created_at: row.get(5)?,
            };
            value.recorded = hil_trust::project_evidence(
                &mut value.source,
                &mut value.status,
                &mut value.summary,
            );
            Ok(value)
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn transition_allowed(from: &str, to: &str) -> bool {
    let a = VALID_STATES.iter().position(|v| *v == from);
    let b = VALID_STATES.iter().position(|v| *v == to);
    matches!((a,b),(Some(x),Some(y)) if y==x+1)
        || matches!(
            (from, to),
            (
                "collecting_evidence" | "verifying" | "reflecting",
                "practicing"
            )
        )
}

#[command]
pub async fn learning_transition_step(
    session_id: String,
    next_state: String,
    answer: Option<String>,
    reflection: Option<String>,
    state: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<LocalLearningSession, String> {
    freshness::transition(
        &state, &workbench, session_id, next_state, answer, reflection,
    )
    .await
}

fn transition_step(
    db: &mut Connection,
    session_id: &str,
    next_state: &str,
    answer: Option<String>,
    reflection: Option<String>,
) -> Result<LocalLearningSession, String> {
    if !VALID_STATES.contains(&next_state) {
        return Err("LEARNING_STATE_INVALID".into());
    }
    let session = get_session(db, session_id)?;
    let (manifest, lab) = manifest_and_lab(db, &session)?;
    prerequisites::require(db, &session, &manifest, &lab.step_id)?;
    let reflection_activity = theory::is_reflection(&lab);
    if !theory::transition(&lab, &session.state, next_state) {
        return Err("LEARNING_STATE_TRANSITION_INVALID".into());
    }
    if answer
        .as_deref()
        .is_some_and(|value| value.chars().count() > 4000)
        || reflection
            .as_deref()
            .is_some_and(|value| value.chars().count() > 4000)
    {
        return Err("LEARNING_TEXT_TOO_LARGE".into());
    }
    if session.state == "checking_understanding"
        && matches!(next_state, "practicing" | "reflecting")
        && answer.as_deref().unwrap_or_default().trim().is_empty()
    {
        return Err("LEARNING_ANSWER_REQUIRED".into());
    }
    if reflection_activity
        && next_state == "completed"
        && answer
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err("LEARNING_ANSWER_REQUIRED".into());
    }
    if next_state == "practicing" && session.project_id <= 0 {
        return Err("LEARNING_PROJECT_REQUIRED".into());
    }
    if session.state == "practicing" && next_state == "collecting_evidence" {
        return Err("LEARNING_ACTION_REQUIRED".into());
    }
    if matches!(next_state, "verifying" | "reflecting" | "completed")
        && !(reflection_activity && next_state == "reflecting")
    {
        require_current_verification(db, &session)?;
    }
    if next_state == "completed" && reflection.as_deref().unwrap_or_default().trim().is_empty() {
        return Err("LEARNING_REFLECTION_REQUIRED".into());
    }
    let answer = answer
        .as_deref()
        .map(crate::supervisor::redact_learning_context);
    let reflection = reflection
        .as_deref()
        .map(crate::supervisor::redact_learning_context);
    let timestamp = now();
    let tx = db.transaction().map_err(|e| e.to_string())?;
    if matches!(
        session.state.as_str(),
        "collecting_evidence" | "verifying" | "reflecting"
    ) && next_state == "practicing"
    {
        let next_attempt = current_attempt(&tx, session_id, &session.current_step)? + 1;
        tx.execute(
            "INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES(?1,?2,?3,'practicing',?4,?4)",
            params![session_id, session.current_step, next_attempt, timestamp],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE learning_sessions SET state='practicing',updated_at=?1 WHERE session_id=?2",
            params![timestamp, session_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        return get_session(db, session_id);
    }
    tx.execute("UPDATE learning_attempts SET state=?1,answer=COALESCE(?2,answer),reflection=COALESCE(?3,reflection),updated_at=?4 WHERE session_id=?5 AND step_id=?6 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?5 AND step_id=?6)",params![next_state,answer,reflection,timestamp,session_id,session.current_step]).map_err(|e|e.to_string())?;
    if reflection_activity && next_state == "completed" {
        let saved_answer: String = tx.query_row("SELECT answer FROM learning_attempts WHERE session_id=?1 AND step_id=?2 ORDER BY attempt DESC LIMIT 1", params![session_id,session.current_step], |r| r.get(0)).map_err(|e|e.to_string())?;
        append_evidence(
            &tx,
            &session,
            "reflection",
            "user-asserted",
            "RECORDED",
            "理解回答与复盘已提交；自述完成，未评分。",
            &serde_json::json!({"assessment":"NOT_GRADED","answerSha256":sha256(saved_answer.as_bytes()),"reflectionSha256":sha256(reflection.as_deref().unwrap_or_default().as_bytes())}),
        )?;
    }
    if next_state == "completed" {
        optional::advance(&tx, &session)?;
    } else {
        tx.execute(
            "UPDATE learning_sessions SET state=?1,updated_at=?2 WHERE session_id=?3",
            params![next_state, timestamp, session_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    get_session(db, session_id)
}

fn completion_classification(hil_result: &str) -> &'static str {
    match hil_result {
        "PASS" => "FULL_PASS",
        "NOT_REQUIRED" => "SOFTWARE_PASS",
        _ => "SOFTWARE_PASS_HIL_NOT_VERIFIED",
    }
}

fn latest_verification(
    db: &Connection,
    session_id: &str,
    step_id: &str,
    kind: &str,
    source: &str,
) -> Result<Option<String>, String> {
    db.query_row(
        "SELECT status FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2) AND kind=?3 AND source=?4 ORDER BY id DESC LIMIT 1",
        params![session_id, step_id, kind, source], |row| row.get(0),
    ).optional().map_err(|e| e.to_string())
}

fn require_current_verification(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(), String> {
    let (_, lab) = manifest_and_lab(db, session)?;
    if theory::is_reflection(&lab) {
        return theory::require_answer(db, session);
    }
    if latest_verification(
        db,
        &session.session_id,
        &session.current_step,
        "command",
        "software",
    )?
    .as_deref()
        != Some("PASS")
    {
        return Err("LEARNING_SOFTWARE_VERIFICATION_REQUIRED".into());
    }
    let (_, lab) = manifest_and_lab(db, session)?;
    if lab.hil_verification.is_some() {
        // Legacy manual aggregates are historical claims, not device receipts.
        return Err("LEARNING_HIL_VERIFICATION_REQUIRED:需要可信设备适配器证据；人工观察与旧规则通过不能替代".into());
    }
    Ok(())
}

fn hil_report_status(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
) -> Result<(String, usize, usize), String> {
    let (_, _, required) = recorded_hil_report_status(db, session, manifest)?;
    // No current local HIL path produces authenticated device receipts.
    Ok((
        if required == 0 {
            "NOT_REQUIRED"
        } else {
            "NOT_VERIFIED"
        }
        .into(),
        0,
        required,
    ))
}

fn recorded_hil_report_status(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
) -> Result<(String, usize, usize), String> {
    let scope = prerequisites::required_scope(manifest);
    let required = manifest
        .labs
        .iter()
        .filter(|lab| scope.contains(&lab.step_id) && lab.hil_verification.is_some())
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Ok(("NOT_REQUIRED".into(), 0, 0));
    }
    let mut passed = 0usize;
    let mut failed = false;
    for lab in &required {
        let status = latest_verification(
            db,
            &session.session_id,
            &lab.step_id,
            "hil-verification",
            "hil-validator",
        )?;
        match status.as_deref() {
            Some("PASS") => passed += 1,
            Some("FAIL") => failed = true,
            _ => {}
        }
    }
    let status = if passed == required.len() {
        "PASS"
    } else if failed {
        "FAIL"
    } else {
        "NOT_VERIFIED"
    };
    Ok((status.into(), passed, required.len()))
}

fn manifest_and_lab(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(LocalCourseManifest, LocalLab), String> {
    let json: String = db
        .query_row(
            "SELECT manifest_json FROM local_courses WHERE course_id=?1 AND version=?2",
            params![session.course_id, session.course_version],
            |r| r.get(0),
        )
        .map_err(|_| "LOCAL_COURSE_NOT_INSTALLED")?;
    let manifest: LocalCourseManifest =
        serde_json::from_str(&json).map_err(|_| "LOCAL_COURSE_MANIFEST_INVALID")?;
    let lab = manifest
        .labs
        .iter()
        .find(|v| v.step_id == session.current_step)
        .cloned()
        .ok_or("LEARNING_STEP_NOT_FOUND")?;
    Ok((manifest, lab))
}

fn append_evidence(
    db: &Connection,
    session: &LocalLearningSession,
    kind: &str,
    source: &str,
    status: &str,
    summary: &str,
    detail: &serde_json::Value,
) -> Result<String, String> {
    let detail_json = detail.to_string();
    let digest = sha256(detail_json.as_bytes());
    let attempt: i64 = db
        .query_row(
            "SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2",
            params![session.session_id, session.current_step],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    db.execute("INSERT INTO learning_evidence(session_id,step_id,attempt,state,kind,source,status,summary,digest,detail_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![session.session_id,session.current_step,attempt,session.state,kind,source,status,summary,digest,detail_json,now()]).map_err(|e|e.to_string())?;
    Ok(digest)
}

fn current_attempt(db: &Connection, session_id: &str, step_id: &str) -> Result<i64, String> {
    db.query_row(
        "SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2",
        params![session_id, step_id],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

fn bounded_redacted_tail(value: &str) -> String {
    diagnostics::tail(value).0
}

#[command]
pub fn learning_prepare_hil_action(
    session_id: String,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<HilPreparation, String> {
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &session_id)?;
    if session.state != "practicing" {
        return Err("LEARNING_STEP_NOT_PRACTICING".into());
    }
    let (_, lab) = manifest_and_lab(&db, &session)?;
    let hil = lab.hil_verification.ok_or("LEARNING_HIL_NOT_REQUIRED")?;
    let attempt = current_attempt(&db, &session.session_id, &session.current_step)?;
    let software_status: Option<String> = db
        .query_row(
            "SELECT status FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1",
            params![session.session_id, session.current_step, attempt],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if software_status.as_deref() != Some("PASS") {
        return Err("LEARNING_SOFTWARE_VERIFICATION_REQUIRED".into());
    }
    let device_id = session.device_id.ok_or("LEARNING_DEVICE_REQUIRED")?;
    let work = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    let project = crate::workbench::storage::project_by_id(&work, session.project_id)
        .map_err(|_| "LEARNING_PROJECT_NOT_FOUND")?;
    if project.device_id != Some(device_id) {
        return Err("LEARNING_DEVICE_SCOPE_CHANGED".into());
    }
    let device = crate::workbench::storage::device_by_id(&work, device_id)
        .map_err(|_| "LEARNING_DEVICE_NOT_FOUND")?;
    if device.board_uid != session.board_uid || session.board_uid.trim().is_empty() {
        return Err("LEARNING_BOARD_UID_CHANGED".into());
    }
    drop(work);
    let token = format!(
        "{:016x}{:016x}{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let token_hash = sha256(token.as_bytes());
    let expires_at = now() + 600;
    let target_json = serde_json::json!({
        "actionId": hil.action_id,
        "target": hil.target,
        "deviceId": device_id,
        "boardUid": session.board_uid,
        "image": hil.image.clone(),
    })
    .to_string();
    db.execute(
        "INSERT INTO learning_hil_approvals(token_hash,session_id,step_id,attempt,action_id,device_id,board_uid,target_json,expires_at,used_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL)",
        params![token_hash, session.session_id, session.current_step, attempt, hil.action_id, device_id, session.board_uid, target_json, expires_at],
    )
    .map_err(|e| e.to_string())?;
    let confirmation_phrase = format!("CONFIRM {} {}", session.board_uid, hil.action_id);
    Ok(HilPreparation {
        action_id: hil.action_id,
        target: hil.target,
        instructions: hil.instructions,
        device_id,
        board_uid: session.board_uid,
        confirmation_phrase,
        approval_token: token,
        expires_at,
        required_evidence_kinds: hil.required_evidence_kinds,
        expected_observations: hil.expected_observations,
        minimum_duration_seconds: hil.minimum_duration_seconds,
        image: hil.image,
    })
}

#[command]
pub fn learning_confirm_hil_action(
    request: HilConfirmationRequest,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<HilApprovalResult, String> {
    if request.approval_token.len() != 64 {
        return Err("LEARNING_HIL_TOKEN_INVALID".into());
    }
    let token_hash = sha256(request.approval_token.as_bytes());
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &request.session_id)?;
    if session.state != "practicing" {
        return Err("LEARNING_STEP_NOT_PRACTICING".into());
    }
    let approval: (String, i64, String, String, i64, Option<i64>) = db
        .query_row(
            "SELECT step_id,attempt,action_id,board_uid,expires_at,used_at FROM learning_hil_approvals WHERE token_hash=?1 AND session_id=?2",
            params![token_hash, session.session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|_| "LEARNING_HIL_TOKEN_INVALID")?;
    let attempt = current_attempt(&db, &session.session_id, &session.current_step)?;
    if approval.0 != session.current_step || approval.1 != attempt || approval.4 < now() {
        return Err("LEARNING_HIL_TOKEN_EXPIRED".into());
    }
    if approval.5.is_some() {
        return Err("LEARNING_HIL_TOKEN_USED".into());
    }
    let expected = format!("CONFIRM {} {}", approval.3, approval.2);
    if request.confirmation.trim() != expected {
        return Err("LEARNING_HIL_CONFIRMATION_MISMATCH".into());
    }
    let updated = db
        .execute(
            "UPDATE learning_hil_approvals SET used_at=?1 WHERE token_hash=?2 AND used_at IS NULL",
            params![now(), token_hash],
        )
        .map_err(|e| e.to_string())?;
    if updated != 1 {
        return Err("LEARNING_HIL_TOKEN_USED".into());
    }
    let (_, lab) = manifest_and_lab(&db, &session)?;
    let hil = lab.hil_verification.ok_or("LEARNING_HIL_NOT_REQUIRED")?;
    let digest = append_evidence(
        &db,
        &session,
        "hil-approval",
        "human",
        "APPROVED",
        "Student confirmed the observation scope; no hardware action was executed",
        &serde_json::json!({
            "actionId": approval.2,
            "boardUid": approval.3,
            "tokenHash": token_hash,
            "confirmationSha256": sha256(request.confirmation.as_bytes()),
        }),
    )?;
    Ok(HilApprovalResult {
        status: "APPROVED".into(),
        action_id: hil.action_id,
        board_uid: session.board_uid,
        evidence_digest: digest,
        instructions: hil.instructions,
    })
}

#[command]
pub fn learning_attach_hil_evidence(
    request: HilEvidenceRequest,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LearningEvidenceView, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    attach_hil_observation(&db, &state.actions, request)
}

fn attach_hil_observation(
    db: &Connection,
    actions: &execution::ActionGate,
    request: HilEvidenceRequest,
) -> Result<LearningEvidenceView, String> {
    if !matches!(request.status.as_str(), "PASS" | "FAIL")
        || request.summary.trim().is_empty()
        || request.summary.chars().count() > 500
        || request.observation.trim().is_empty()
        || request.observation.chars().count() > 8000
    {
        return Err("LEARNING_HIL_EVIDENCE_INVALID".into());
    }
    let session = get_session(db, &request.session_id)?;
    hil_trust::context(db, &session, &request)?;
    let _lease = actions.acquire(session.project_id)?;
    if session.state != "practicing" {
        return Err("LEARNING_STEP_NOT_PRACTICING".into());
    }
    let (_, lab) = manifest_and_lab(db, &session)?;
    let hil = lab.hil_verification.ok_or("LEARNING_HIL_NOT_REQUIRED")?;
    let attempt = current_attempt(db, &session.session_id, &session.current_step)?;
    if !hil
        .required_evidence_kinds
        .iter()
        .any(|kind| kind == &request.evidence_kind)
    {
        return Err("LEARNING_HIL_EVIDENCE_KIND_FORBIDDEN".into());
    }
    let approvals: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM learning_hil_approvals WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND action_id=?4 AND used_at IS NOT NULL",
            params![session.session_id, session.current_step, attempt, hil.action_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if approvals == 0 {
        return Err("LEARNING_HIL_APPROVAL_REQUIRED".into());
    }
    let artifact_sha256 = request
        .artifact_sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if artifact_sha256.is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err("LEARNING_HIL_ARTIFACT_HASH_INVALID".into());
    }
    if request.status == "PASS" && request.evidence_kind == "flash-verification" {
        let expected = hil.image.as_ref().ok_or("LEARNING_HIL_IMAGE_MISSING")?;
        let actual_sha256 = artifact_sha256.map(str::to_ascii_lowercase);
        let expected_sha256 = expected.sha256.to_ascii_lowercase();
        if actual_sha256.as_deref() != Some(expected_sha256.as_str()) {
            return Err("LEARNING_HIL_IMAGE_HASH_MISMATCH".into());
        }
    }
    if request.status == "PASS"
        && request.evidence_kind == "soak-observation"
        && request.duration_seconds.unwrap_or_default()
            < hil.minimum_duration_seconds.unwrap_or_default()
    {
        return Err("LEARNING_HIL_DURATION_INSUFFICIENT".into());
    }
    let created_at = now();
    let summary = crate::supervisor::redact_learning_context(&format!(
        "人工观察（声称 {}，未独立验证）：{}",
        request.status,
        request.summary.trim()
    ));
    let detail = serde_json::json!({
        "claimedStatus": request.status,
        "trust": "user-asserted",
        "actionId": hil.action_id,
        "boardUid": session.board_uid,
        "observation": bounded_redacted_tail(&request.observation),
        "observationSha256": sha256(request.observation.as_bytes()),
        "artifactSha256": artifact_sha256,
        "durationSeconds": request.duration_seconds,
    });
    let digest = append_evidence(
        db,
        &session,
        &request.evidence_kind,
        "user-asserted",
        "RECORDED",
        &summary,
        &detail,
    )?;
    Ok(LearningEvidenceView {
        recorded: None,
        kind: request.evidence_kind,
        source: "user-asserted".into(),
        status: "RECORDED".into(),
        summary,
        digest,
        created_at,
    })
}

#[command]
pub fn learning_finalize_hil_step(
    session_id: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<StepActionResult, String> {
    let mut db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    hil_trust::assessment(&mut db, &state.actions, &session_id)
}

fn ensure_action_context(
    db: &Connection,
    original: &LocalLearningSession,
    attempt: i64,
) -> Result<(), String> {
    let current = get_session(db, &original.session_id)?;
    if current.current_step != original.current_step
        || current.state != original.state
        || current_attempt(db, &original.session_id, &original.current_step)? != attempt
    {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    Ok(())
}

fn mark_collecting(db: &Connection, session: &LocalLearningSession) -> Result<(), String> {
    let timestamp = now();
    db.execute("UPDATE learning_sessions SET state='collecting_evidence',updated_at=?1 WHERE session_id=?2",params![timestamp,session.session_id]).map_err(|e| e.to_string())?;
    db.execute("UPDATE learning_attempts SET state='collecting_evidence',updated_at=?1 WHERE session_id=?2 AND step_id=?3 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?2 AND step_id=?3)",params![timestamp,session.session_id,session.current_step]).map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
pub async fn learning_run_step_action(
    session_id: String,
    credentials: crate::workbench::types::SshCredentials,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<StepActionResult, String> {
    let (
        session,
        attempt,
        lease,
        lab,
        project,
        host,
        remote_subdir,
        input_paths,
        prerequisite_inputs,
    ) = {
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        let session = get_session(&db, &session_id)?;
        if session.state != "practicing" {
            return Err("LEARNING_STEP_NOT_PRACTICING".into());
        }
        let (manifest, lab) = manifest_and_lab(&db, &session)?;
        if theory::is_reflection(&lab) {
            return Err("LEARNING_ACTIVITY_HAS_NO_EXECUTION".into());
        }
        let work = workbench
            .db
            .lock()
            .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
        let project = crate::workbench::storage::project_by_id(&work, session.project_id)
            .map_err(|_| "LEARNING_PROJECT_NOT_FOUND")?;
        let host = crate::workbench::storage::host_by_id(&work, project.host_id)
            .map_err(|_| "LEARNING_HOST_NOT_FOUND")?;
        let attempt = current_attempt(&db, &session_id, &session.current_step)?;
        let lease = local.actions.acquire(session.project_id)?;
        freshness::current_binding(&work, &session)?;
        let input_paths = freshness::inputs(&db, &session, &manifest)?;
        let prerequisite_inputs = prerequisites::execution_plan(&db, &session, &manifest, &lab)?;
        (
            session,
            attempt,
            lease,
            lab,
            project,
            host,
            manifest
                .workspace
                .map(|w| w.remote_subdir)
                .unwrap_or_else(|| "tutorial-examples/web-kvm".into()),
            input_paths,
            prerequisite_inputs,
        )
    };
    let binding_digest = freshness::binding(&project, &host)?;
    let execution_binding = binding_digest.clone();
    let rule = lab
        .verification_rules
        .first()
        .ok_or("LEARNING_VERIFICATION_MISSING")?
        .clone();
    let literal_command = execution::render_command(&rule.command)?;
    let resolved = crate::workbench::credentials::resolve(host.id, credentials)?;
    let sdk_dir = project.sdk_dir.clone();
    let artifact_specs = lab.artifact_outputs.clone();
    let environment_specs = lab.environment_checks.clone();
    // The worker owns the lease even if its async caller is dropped. Return it with
    // the result so a second action cannot start before the evidence transaction.
    let (_lease, output) = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let (ssh, _, _) = crate::workbench::ssh::connect(&host, &resolved)?;
            let root = format!("{}/{}", sdk_dir.trim_end_matches('/'), remote_subdir);
            let before = freshness::observe(&ssh, &root)?;
            if !prerequisite_inputs.is_empty() {
                prerequisites::check_with(
                    &prerequisite_inputs,
                    &freshness::source_identity(&before, &input_paths)?,
                    &execution_binding,
                    |specs| environment_receipt::checked(&ssh, &sdk_dir, specs),
                )?;
            }
            let environment_before =
                environment_receipt::checked(&ssh, &sdk_dir, &environment_specs)?;
            let deadline = std::time::Instant::now() + execution::COMMAND_BUDGET;
            let check_budget = || {
                ssh.set_timeout(execution::remaining_timeout(
                    deadline,
                    std::time::Instant::now(),
                )?);
                Ok(())
            };
            check_budget()?;
            let mut channel = ssh.channel_session().map_err(|_| "LEARNING_SSH_CHANNEL")?;
            check_budget()?;
            channel
                .handle_extended_data(ssh2::ExtendedData::Merge)
                .map_err(|_| "LEARNING_SSH_STREAM")?;
            let inner = format!(
                "cd {} && {}",
                crate::workbench::ssh::shell_quote(&root),
                literal_command
            );
            let command = format!("bash -lc {}", crate::workbench::ssh::shell_quote(&inner));
            check_budget()?;
            channel.exec(&command).map_err(|_| "LEARNING_SSH_EXEC")?;
            let bytes =
                execution::read_output(&mut channel, execution::MAX_OUTPUT_BYTES, check_budget)?;
            check_budget()?;
            channel.wait_close().map_err(|_| "LEARNING_SSH_CLOSE")?;
            check_budget()?;
            let exit = channel.exit_status().map_err(|_| "LEARNING_SSH_STATUS")?;
            let after = freshness::observe(&ssh, &root)?;
            freshness::unchanged_inputs(&before, &after, &input_paths)?;
            if !prerequisite_inputs.is_empty() {
                prerequisites::check_with(
                    &prerequisite_inputs,
                    &freshness::source_identity(&after, &input_paths)?,
                    &execution_binding,
                    |specs| environment_receipt::checked(&ssh, &sdk_dir, specs),
                )?;
            }
            let environment_after =
                environment_receipt::checked(&ssh, &sdk_dir, &environment_specs)?;
            environment_receipt::compare(
                &environment_before,
                &environment_after,
                &environment_specs,
            )?;
            let snapshot = freshness::freeze_sources(
                &after,
                &execution_binding,
                environment_after,
                &input_paths,
            )?;
            let artifacts = if exit == rule.expected_exit_code {
                artifacts::capture(&ssh, &after, &artifact_specs)?
            } else {
                Vec::new()
            };
            Ok::<_, String>((exit, bytes, rule.expected_exit_code, snapshot, artifacts))
        })();
        (lease, result)
    })
    .await
    .map_err(|_| "LEARNING_ACTION_TASK_FAILED".to_owned())?;
    let mut db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let work = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    if freshness::current_binding(&work, &session)? != binding_digest {
        let error = "LEARNING_INPUTS_CHANGED:执行期间工程绑定已变化，请重新实践";
        record_action_failure(&db, &session, attempt, error)?;
        return Err(error.into());
    }
    match output {
        Ok((exit, bytes, expected, snapshot, artifacts)) => record_observed_action_result(
            &mut db,
            &session,
            attempt,
            exit,
            expected,
            &bytes,
            ActionObservation {
                has_hil: lab.hil_verification.is_some(),
                snapshot: Some(snapshot),
                artifacts,
            },
        ),
        Err(error) => {
            record_action_failure(&db, &session, attempt, &error)?;
            Err(error)
        }
    }
}

fn record_action_failure(
    db: &Connection,
    session: &LocalLearningSession,
    attempt: i64,
    error: &str,
) -> Result<(), String> {
    ensure_action_context(db, session, attempt)?;
    let (text, truncated) = diagnostics::tail(error);
    append_evidence(
        db,
        session,
        "command",
        "software",
        "ERROR",
        "课程命令未完成；请检查远端任务状态后再手动重试",
        &serde_json::json!({"diagnosticVersion":1,"error":text,"outputTruncated":truncated}),
    )?;
    Ok(())
}

#[cfg(test)]
fn record_action_result(
    db: &mut Connection,
    session: &LocalLearningSession,
    attempt: i64,
    exit: i32,
    expected: i32,
    bytes: &[u8],
    has_hil: bool,
) -> Result<StepActionResult, String> {
    record_observed_action_result(
        db,
        session,
        attempt,
        exit,
        expected,
        bytes,
        ActionObservation {
            has_hil,
            snapshot: None,
            artifacts: Vec::new(),
        },
    )
}

struct ActionObservation {
    has_hil: bool,
    snapshot: Option<freshness::Snapshot>,
    artifacts: Vec<artifacts::Collected>,
}
fn record_observed_action_result(
    db: &mut Connection,
    session: &LocalLearningSession,
    attempt: i64,
    exit: i32,
    expected: i32,
    bytes: &[u8],
    observation: ActionObservation,
) -> Result<StepActionResult, String> {
    let ActionObservation {
        has_hil,
        snapshot,
        artifacts,
    } = observation;
    if exit != expected && !artifacts.is_empty() {
        return Err("LEARNING_ARTIFACT_INVALID:失败命令不能保存成功产物快照".into());
    }
    let text = String::from_utf8_lossy(bytes);
    let status = if exit == expected { "PASS" } else { "FAIL" };
    let source = "software";
    let tx = db.transaction().map_err(|e| e.to_string())?;
    ensure_action_context(&tx, session, attempt)?;
    let raw_output_sha256 = sha256(bytes);
    let (text, truncated) = diagnostics::tail(&text);
    let digest = append_evidence(
        &tx,
        session,
        "command",
        source,
        status,
        "Manifest allowlisted action completed",
        &serde_json::json!({"diagnosticVersion":1,"exitCode":exit,"expectedExitCode":expected,"outputSha256":raw_output_sha256,"outputTail":text,"outputTruncated":truncated,"workspaceSnapshot":snapshot,"artifactVersion":1,"artifacts":artifacts.iter().map(|a|&a.meta).collect::<Vec<_>>()}),
    )?;
    artifacts::store(&tx, tx.last_insert_rowid(), &artifacts)?;
    if status == "PASS" && !has_hil {
        mark_collecting(&tx, session)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(StepActionResult {
        status: status.into(),
        source: source.into(),
        exit_code: Some(exit),
        output_digest: digest,
        summary: if status == "PASS" && has_hil {
            "软件校验通过；继续完成逐次确认和真实 HIL 取证".into()
        } else if status == "PASS" {
            "确定性命令通过".into()
        } else {
            "命令退出码不符合课程规则".into()
        },
    })
}

#[command]
pub async fn learning_ask_local_tutor(
    request: LocalTutorRequest,
    local: State<'_, Arc<LocalLearningState>>,
    supervisor: State<'_, Arc<SupervisorState>>,
) -> Result<LocalTutorAnswer, String> {
    let (session, attempt, context) = {
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        tutor_context::prepare(&db, &request)?
    };
    if cfg!(test) && std::env::var("LYNX_LEARNING_TUTOR_FIXTURE").as_deref() == Ok("1") {
        let result = LocalTutorAnswer {
            answer: "固定测试助教：先核对当前步骤的输入、输出和证据来源。".into(),
            context_scope: tutor_context::scope(request.diagnostic_digest.is_some()),
            executed_action: false,
            fixture: true,
        };
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        journey::ensure_agent_context(&db, &session, attempt)?; // LEARN_AGENT_CONTEXT_CHANGED
        append_evidence(
            &db,
            &session,
            "tutor",
            "ai-fixture",
            "RECORDED",
            "Deterministic tutor hint recorded",
            &tutor_context::record_detail(&request, &result.answer),
        )?;
        return Ok(result);
    }
    let (provider, key) =
        crate::supervisor::commands::resolve_default_provider(supervisor.inner().as_ref())?;
    if !provider.enabled {
        return Err("AI_PROVIDER_DISABLED".into());
    }
    let body = TutorBody {
        model: provider.model.clone(),
        messages: vec![
            TutorMessage {
                role: "system",
                content: tutor_context::SYSTEM_PROMPT.into(),
            },
            TutorMessage {
                role: "user",
                content: context.to_string(),
            },
        ],
        max_tokens: provider.max_tokens.unwrap_or(800).clamp(64, 800),
        stream: false,
    };
    let response = supervisor
        .http
        .post(crate::supervisor::commands::endpoint(
            &provider.base_url,
            "chat/completions",
        )?)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(
            provider.timeout_seconds.clamp(5, 120),
        ))
        .json(&body)
        .send()
        .await
        .map_err(|_| "LEARNING_TUTOR_NETWORK")?;
    if !response.status().is_success() {
        return Err(crate::supervisor::commands::safe_http_message(
            response.status(),
        ));
    }
    let parsed = response
        .json::<TutorResponse>()
        .await
        .map_err(|_| "LEARNING_TUTOR_RESPONSE_INVALID")?;
    let answer = parsed
        .choices
        .first()
        .map(|v| v.message.content.trim())
        .filter(|v| !v.is_empty())
        .ok_or("LEARNING_TUTOR_RESPONSE_EMPTY")?;
    let result = LocalTutorAnswer {
        answer: answer.chars().take(8000).collect(),
        context_scope: tutor_context::scope(request.diagnostic_digest.is_some()),
        executed_action: false,
        fixture: false,
    };
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    journey::ensure_agent_context(&db, &session, attempt)?; // LEARN_AGENT_CONTEXT_CHANGED
    append_evidence(
        &db,
        &session,
        "tutor",
        "ai-provider",
        "RECORDED",
        "AI tutor hint recorded",
        &tutor_context::record_detail(&request, &result.answer),
    )?;
    Ok(result)
}

#[command]
pub async fn learning_generate_local_report(
    session_id: String,
    state: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<LearningReport, String> {
    report::current(&state, &workbench, &session_id).await
}

fn generate_local_report(db: &Connection, session_id: &str) -> Result<LearningReport, String> {
    let session = get_session(db, session_id)?;
    let (manifest, _) = manifest_and_lab(db, &session)?;
    let prerequisite_checks = prerequisites::evaluate(db, &session, &manifest)?;
    let required_scope = prerequisites::required_scope(&manifest);
    let mut completed = 0;
    let mut software_passed = 0;
    let mut required_completed_steps = 0;
    let mut optional_completed_steps = 0;
    let mut optional_skipped_steps = 0;
    let required_steps = manifest.labs.iter().filter(|lab| lab.required).count();
    let required_practice = manifest
        .labs
        .iter()
        .filter(|lab| required_scope.contains(&lab.step_id) && !theory::is_reflection(lab))
        .count();
    for lab in &manifest.labs {
        let latest_state: Option<String> = db.query_row("SELECT state FROM learning_attempts WHERE session_id=?1 AND step_id=?2 ORDER BY attempt DESC LIMIT 1", params![session_id,lab.step_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        if latest_state.as_deref() == Some("completed") {
            completed += 1;
            if lab.required {
                required_completed_steps += 1;
            } else {
                optional_completed_steps += 1;
            }
            if required_scope.contains(&lab.step_id)
                && !theory::is_reflection(lab)
                && latest_verification(db, session_id, &lab.step_id, "command", "software")?
                    .as_deref()
                    == Some("PASS")
            {
                software_passed += 1;
            }
        } else if !lab.required && latest_state.as_deref() == Some("skipped") {
            optional_skipped_steps += 1;
        }
    }
    let evidence: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM learning_evidence WHERE session_id=?1",
            [&session_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let (hil_result, hil_passed_steps, hil_required_steps) =
        hil_report_status(db, &session, &manifest)?;
    let mut report = LearningReport {
        prerequisite_checks,
        required_steps,
        required_completed_steps,
        optional_steps: manifest.labs.len() - required_steps,
        optional_completed_steps,
        optional_skipped_steps,
        session,
        completed_steps: completed,
        total_steps: manifest.labs.len(),
        evidence_count: evidence as usize,
        software_result: if required_practice == 0 {
            "NOT_REQUIRED".into()
        } else if software_passed == required_practice {
            "PASS".into()
        } else {
            "INCOMPLETE".into()
        },
        hil_result,
        recorded_hil_result: recorded_hil_report_status(
            db,
            &get_session(db, session_id)?,
            &manifest,
        )?
        .0,
        hil_passed_steps,
        hil_required_steps,
        provenance_result: "PASS".into(),
        recorded_software_result: String::new(),
        current_workspace: report::initial(required_practice == 0),
    };
    report.recorded_software_result = report.software_result.clone();
    for access in &report.prerequisite_checks {
        if access.completed {
            continue;
        }
        let lab = manifest
            .labs
            .iter()
            .find(|l| l.step_id == access.step_id)
            .ok_or("LEARNING_STEP_NOT_FOUND")?;
        if revisit::latest(db, session_id, &lab.step_id)?
            .is_some_and(|(_, state)| state == "completed")
        {
            report.completed_steps -= 1;
            if lab.required {
                report.required_completed_steps -= 1;
            } else {
                report.optional_completed_steps -= 1;
            }
        }
        if required_scope.contains(&lab.step_id) && !theory::is_reflection(lab) {
            report.software_result = "INCOMPLETE".into();
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn transitions_are_strict() {
        assert!(transition_allowed("reading", "checking_understanding"));
        assert!(transition_allowed("verifying", "practicing"));
        assert!(!transition_allowed("reading", "completed"));
    }
    #[test]
    fn commands_reject_substitution_and_quoted_traversal() {
        for bad in [
            "./scripts/check.sh $(printf harmless)",
            "./scripts/check.sh $HOME",
            "./scripts/*.sh source",
            "./scripts/.'./'outside",
            "./scripts/check.sh 'unterminated",
            "./scripts/check.sh arg\0suffix",
        ] {
            assert!(
                validate_command(bad).is_err(),
                "accepted unsafe rule: {bad:?}"
            );
        }
    }

    #[test]
    fn commands_are_manifest_scoped() {
        assert!(validate_command("./scripts/run_simulation.sh video-socket").is_ok());
        for bad in [
            "make all",
            "./scripts/x; reboot",
            "./scripts/../bad",
            "./scripts/x | sh",
        ] {
            assert!(validate_command(bad).is_err());
        }
    }
    #[test]
    fn path_traversal_is_rejected() {
        assert!(safe_relative("workspace/file").is_ok());
        assert!(safe_relative("../secret").is_err());
        assert!(safe_relative("C:\\secret").is_err());
    }

    pub(super) fn manifest_v2_with_hil() -> LocalCourseManifest {
        let labs = (1..=24)
            .map(|number| LocalLab {
                prerequisites: None,
                environment_checks: Vec::new(),
                required: true,
                artifact_outputs: Vec::new(),
                activity_kind: "practice".into(),
                step_id: format!("step-{number}"),
                number,
                path: format!("lesson-{number}"),
                title: format!("Lesson {number}"),
                learning_objective: "objective".into(),
                comprehension_check: "check".into(),
                student_action: "action".into(),
                risk_level: if number == 2 {
                    "hardware-confirmation".into()
                } else {
                    "software-safe".into()
                },
                expected_observation: "expected".into(),
                verification_rules: vec![VerificationRule {
                    id: "exit-zero".into(),
                    kind: "command".into(),
                    command: "./scripts/check.sh".into(),
                    expected_exit_code: 0,
                }],
                required_evidence: vec![EvidenceRequirement {
                    kind: "command".into(),
                    source: "software".into(),
                }],
                reflection_prompt: "reflect".into(),
                document: None,
                hil_verification: (number == 2).then(|| HilVerification {
                    action_id: "verify-wiring".into(),
                    target: "avaota-a1-t527-lt6911c".into(),
                    instructions: "inspect".into(),
                    required_evidence_kinds: vec!["wiring-observation".into()],
                    expected_observations: vec!["wired".into()],
                    minimum_duration_seconds: None,
                    image: None,
                }),
            })
            .collect();
        LocalCourseManifest {
            schema_version: 2,
            course_id: "t527-course".into(),
            version: "1.0.0-rc.1".into(),
            title: "T527".into(),
            board_profile: "avaota-a1-t527-lt6911c".into(),
            provenance_sha256: "0".repeat(64),
            workspace: None,
            labs,
        }
    }

    #[test]
    fn manifest_v3_supports_host_only_courses_with_scoped_workspace() {
        let mut manifest = manifest_v2_with_hil();
        manifest.schema_version = 3;
        manifest.labs.truncate(1);
        manifest.labs[0].hil_verification = None;
        assert!(validate_manifest(&manifest).is_err());
        manifest.workspace = Some(LearnWorkspaceSpec {
            remote_subdir: "courses/linux-app".into(),
            package_source_root: "workspace/linux-app".into(),
        });
        assert!(validate_manifest(&manifest).is_ok());
        manifest.workspace.as_mut().unwrap().remote_subdir = "../other".into();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn manifest_v2_requires_scoped_hil_and_keeps_v1_compatible() {
        let manifest = manifest_v2_with_hil();
        assert!(validate_manifest(&manifest).is_ok());
        let mut wrong_target = manifest.clone();
        wrong_target.labs[1]
            .hil_verification
            .as_mut()
            .unwrap()
            .target = "another-board".into();
        assert_eq!(
            validate_manifest(&wrong_target).unwrap_err(),
            "LOCAL_COURSE_HIL_INVALID"
        );
        let mut v1 = manifest;
        v1.schema_version = 1;
        v1.labs[1].hil_verification = None;
        assert!(validate_manifest(&v1).is_ok());
    }

    #[test]
    fn migration_adds_device_binding_and_single_use_hil_store() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE learning_sessions(session_id TEXT PRIMARY KEY,course_id TEXT NOT NULL,course_version TEXT NOT NULL,project_id INTEGER NOT NULL,current_step TEXT NOT NULL,state TEXT NOT NULL,classification TEXT NOT NULL DEFAULT 'INCOMPLETE',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
             CREATE TABLE learning_evidence(id INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,step_id TEXT NOT NULL,attempt INTEGER NOT NULL,kind TEXT NOT NULL,source TEXT NOT NULL,status TEXT NOT NULL,summary TEXT NOT NULL,digest TEXT NOT NULL,detail_json TEXT NOT NULL,created_at INTEGER NOT NULL);",
        )
        .unwrap();
        migrate(&db).unwrap();
        let session_columns = db
            .prepare("PRAGMA table_info(learning_sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for required in ["device_id", "board_uid", "course_package_sha256"] {
            assert!(session_columns.iter().any(|column| column == required));
        }
        let evidence_columns = db
            .prepare("PRAGMA table_info(learning_evidence)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(evidence_columns.iter().any(|column| column == "state"));
        db.execute(
            "INSERT INTO learning_hil_approvals(token_hash,session_id,step_id,attempt,action_id,device_id,board_uid,target_json,expires_at,used_at) VALUES('token','session','step',1,'action',1,'board','{}',9999999999,NULL)",
            [],
        )
        .unwrap();
        assert_eq!(
            db.execute(
                "UPDATE learning_hil_approvals SET used_at=1 WHERE token_hash='token' AND used_at IS NULL",
                [],
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.execute(
                "UPDATE learning_hil_approvals SET used_at=2 WHERE token_hash='token' AND used_at IS NULL",
                [],
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn signed_envelope_verifies_and_tampering_is_rejected() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let payload = b"deterministic-course-payload";
        let digest = Sha256::digest(payload);
        let envelope = serde_json::json!({
            "format":"lynx-course-v1",
            "algorithm":"Ed25519-SHA256",
            "publicKey":STANDARD.encode(signing.verifying_key().as_bytes()),
            "sha256":hex::encode(digest),
            "signature":STANDARD.encode(signing.sign(&digest).to_bytes()),
            "payload":STANDARD.encode(payload),
        });
        assert!(verify_envelope(envelope.to_string().as_bytes()).is_ok());
        let mut tampered = envelope;
        tampered["payload"] = serde_json::Value::String(STANDARD.encode(b"tampered"));
        assert_eq!(
            verify_envelope(tampered.to_string().as_bytes()).unwrap_err(),
            "LOCAL_COURSE_HASH_MISMATCH"
        );
    }

    #[test]
    fn production_course_fixture_is_rust_compatible_when_provided() {
        let Ok(path) = std::env::var("LYNX_COURSE_PACKAGE_FIXTURE") else {
            return;
        };
        let package = fs::read(path).expect("read production course fixture");
        let (_, payload, _) =
            verify_envelope(&package).expect("verify production course signature");
        let (manifest, provenance) = inspect_zip(&payload).expect("inspect production course");
        assert_eq!(manifest.labs.len(), 24);
        let target = std::env::temp_dir().join(format!(
            "lynx-course-contract-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        extract_zip(&payload, &target, &provenance, &manifest)
            .expect("verify locked course source files");
        fs::remove_dir_all(target).expect("remove production course fixture extraction");
    }
}

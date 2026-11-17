//! Signed author previews and scoped read-integration receipts. No course execution.
use super::{
    courses, error,
    manifest::{self, SignedEnvelope, TrustedKeys, VerifiedCourse},
    schema_error, LearnError,
};
use crate::auth::state::AuthState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tauri::State;
pub mod discovery;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewBinding {
    pub format: String,
    pub schema_version: u32,
    pub preview_id: String,
    pub user_id: i64,
    pub draft_id: String,
    pub draft_revision: i64,
    pub content_digest: String,
    pub course_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub policy_sha256: String,
    pub allowed_unit_ids: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub scope: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResponse {
    content_envelope: SignedEnvelope,
    policy_envelope: SignedEnvelope,
    binding_envelope: SignedEnvelope,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedPreview {
    pub binding: PreviewBinding,
    pub course: VerifiedCourse,
    pub current_unit_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewUnit {
    preview_id: String,
    release_id: String,
    content_digest: String,
    unit_id: String,
    documents: Vec<courses::DocumentSnapshot>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentCheck {
    pub unit_id: String,
    pub resource_id: String,
    pub sha256: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadReceipt {
    pub format: String,
    pub schema_version: u32,
    pub receipt_id: String,
    pub preview_id: String,
    pub user_id: i64,
    pub draft_revision: i64,
    pub content_digest: String,
    pub manifest_sha256: String,
    pub scope: String,
    pub status: String,
    pub client_observation_trust: String,
    pub publication_eligible: bool,
    pub client_version: String,
    pub platform: String,
    pub verified_at: DateTime<Utc>,
    pub documents: Vec<DocumentCheck>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptResponse {
    receipt_id: String,
    receipt_envelope: SignedEnvelope,
}
fn mismatch() -> LearnError {
    error(
        "LEARN_PREVIEW_MISMATCH",
        "试学账号、版本或回执不匹配，请重新打开邀请",
        false,
    )
}
fn verify_preview(
    r: PreviewResponse,
    id: &str,
    user: i64,
    keys: &TrustedKeys,
) -> Result<VerifiedPreview, LearnError> {
    let b: PreviewBinding = manifest::verify(&r.binding_envelope, keys, 65536)?;
    if b.format != "100ask.learn-preview-binding"
        || b.schema_version != 1
        || b.preview_id != id
        || b.user_id != user
        || user <= 0
        || b.scope != "draft_readonly"
        || b.draft_revision < 1
        || !manifest::valid_id(&b.draft_id)
        || b.release_id != id
        || !manifest::valid_digest(&b.content_digest)
        || b.manifest_sha256 != r.content_envelope.sha256
        || b.policy_sha256 != r.policy_envelope.sha256
    {
        return Err(mismatch());
    }
    if b.expires_at <= Utc::now() {
        return Err(error(
            "LEARN_PREVIEW_EXPIRED",
            "试学邀请已过期，请在官网重新生成",
            false,
        ));
    }
    let m: manifest::CourseManifest =
        manifest::verify(&r.content_envelope, keys, manifest::MAX_MANIFEST_BYTES)?;
    let e = manifest::EnrollmentSummary {
        course_slug: None,
        enrollment_id: id.into(),
        course_id: b.course_id.clone(),
        release_id: b.release_id.clone(),
        manifest_sha256: b.manifest_sha256.clone(),
        version: format!("0.0.0-preview.{}", b.draft_revision),
        title: m.title.clone(),
        summary: String::new(),
        track: m.track.clone(),
        current_unit_id: None,
    };
    // Reuse exact signed manifest/policy validation, but never expose a formal enrollment.
    let course = manifest::verify_course(
        manifest::ManifestResponse {
            content_envelope: r.content_envelope,
            policy_envelope: r.policy_envelope,
            entitlement: manifest::Entitlement {
                state: "active".into(),
            },
        },
        &e,
        keys,
    )?;
    let expected: HashSet<_> = course
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .map(|u| u.unit_id.as_str())
        .collect();
    let actual: HashSet<_> = b.allowed_unit_ids.iter().map(String::as_str).collect();
    if expected != actual || actual.len() != b.allowed_unit_ids.len() {
        return Err(mismatch());
    }
    Ok(VerifiedPreview {
        binding: b,
        course,
        current_unit_id: None,
    })
}
async fn preview(
    base: &str,
    token: &str,
    id: &str,
    user: i64,
    keys: &TrustedKeys,
) -> Result<VerifiedPreview, LearnError> {
    let r = courses::read(base, token, &["previews", id, "manifest"]).await?;
    verify_preview(r, id, user, keys)
}
async fn unit(
    base: &str,
    token: &str,
    p: &VerifiedPreview,
    id: &str,
) -> Result<courses::LearnUnitContent, LearnError> {
    if !p.binding.allowed_unit_ids.iter().any(|s| s == id) {
        return Err(mismatch());
    }
    let r: PreviewUnit = courses::read(
        base,
        token,
        &["previews", &p.binding.preview_id, "units", id],
    )
    .await?;
    if r.preview_id != p.binding.preview_id || r.content_digest != p.binding.content_digest {
        return Err(mismatch());
    }
    let c = courses::LearnUnitContent {
        enrollment_id: r.preview_id,
        release_id: r.release_id,
        unit_id: r.unit_id,
        documents: r.documents,
    };
    courses::validate_documents(&c, &p.course, id)?;
    Ok(c)
}
fn verify_receipt(
    r: ReceiptResponse,
    p: &VerifiedPreview,
    docs: &[DocumentCheck],
    keys: &TrustedKeys,
) -> Result<ReadReceipt, LearnError> {
    let v: ReadReceipt = manifest::verify(&r.receipt_envelope, keys, 512 * 1024)?;
    let mut expected = docs.to_vec();
    let mut actual = v.documents.clone();
    expected.sort_by(|a, b| (&a.unit_id, &a.resource_id).cmp(&(&b.unit_id, &b.resource_id)));
    actual.sort_by(|a, b| (&a.unit_id, &a.resource_id).cmp(&(&b.unit_id, &b.resource_id)));
    if v.format != "100ask.learn-preview-read-receipt"
        || v.schema_version != 1
        || v.receipt_id != r.receipt_id
        || !manifest::valid_id(&v.receipt_id)
        || v.preview_id != p.binding.preview_id
        || v.user_id != p.binding.user_id
        || v.draft_revision != p.binding.draft_revision
        || v.content_digest != p.binding.content_digest
        || v.manifest_sha256 != p.binding.manifest_sha256
        || v.scope != "read_integration"
        || v.status != "server_resources_verified"
        || v.client_observation_trust != "client_reported"
        || v.publication_eligible
        || v.client_version != env!("CARGO_PKG_VERSION")
        || v.platform != std::env::consts::OS
        || actual != expected
        || v.verified_at > Utc::now() + chrono::Duration::minutes(1)
        || v.verified_at > p.binding.expires_at
    {
        return Err(mismatch());
    }
    Ok(v)
}
async fn read_check(
    base: &str,
    token: &str,
    p: &VerifiedPreview,
    keys: &TrustedKeys,
    request_id: &str,
) -> Result<ReadReceipt, LearnError> {
    let mut documents = Vec::new();
    let mut bytes = 0usize;
    for id in &p.binding.allowed_unit_ids {
        let c = unit(base, token, p, id).await?;
        for d in c.documents {
            bytes = bytes.saturating_add(d.markdown.len());
            if bytes > 32 * 1024 * 1024 || documents.len() >= 2048 {
                return Err(error(
                    "LEARN_PREVIEW_LIMIT",
                    "教材联调规模超限，请拆分预览",
                    false,
                ));
            }
            documents.push(DocumentCheck {
                unit_id: id.clone(),
                resource_id: d.resource_id,
                sha256: d.sha256,
            });
        }
    }
    let body = serde_json::json!({"contentDigest":p.binding.content_digest,"manifestSha256":p.binding.manifest_sha256,"clientVersion":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"documents":documents});
    let raw = super::post_response(
        courses::url(base, &["previews", &p.binding.preview_id, "read-checks"])?,
        token,
        &body,
        request_id,
        1024 * 1024,
    )
    .await?;
    #[derive(Deserialize)]
    struct Data {
        data: ReceiptResponse,
    }
    let data: Data = serde_json::from_slice(&raw).map_err(|_| schema_error())?;
    verify_receipt(data.data, p, &documents, keys)
}
async fn context(auth: &AuthState) -> Result<(String, String, i64), LearnError> {
    let (base, token) = courses::context(auth).await?;
    let user = auth
        .get_user()
        .await
        .map_err(|_| mismatch())?
        .ok_or_else(mismatch)?
        .id;
    Ok((base, token, user))
}
#[tauri::command]
pub async fn learn_get_preview(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<crate::learning::local::LocalLearningState>>,
    preview_id: String,
) -> Result<VerifiedPreview, LearnError> {
    let (base, token, user) = context(&auth).await?;
    let mut p = preview(
        &base,
        &token,
        &preview_id,
        user,
        &manifest::official_keys()?,
    )
    .await?;
    discovery::save_open(&local, &base, &mut p)?;
    Ok(p)
}
#[tauri::command]
pub async fn learn_get_preview_unit(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<crate::learning::local::LocalLearningState>>,
    preview_id: String,
    content_digest: String,
    unit_id: String,
) -> Result<courses::LearnUnitContent, LearnError> {
    let (base, token, user) = context(&auth).await?;
    let p = preview(
        &base,
        &token,
        &preview_id,
        user,
        &manifest::official_keys()?,
    )
    .await?;
    if p.binding.content_digest != content_digest {
        return Err(mismatch());
    }
    let content = unit(&base, &token, &p, &unit_id).await?;
    discovery::save_unit(&local, &base, &p, &unit_id)?;
    Ok(content)
}
#[tauri::command]
pub async fn learn_check_preview_read(
    auth: State<'_, Arc<AuthState>>,
    preview_id: String,
    content_digest: String,
    request_id: String,
) -> Result<ReadReceipt, LearnError> {
    if !manifest::valid_id(&request_id) {
        return Err(schema_error());
    }
    let (base, token, user) = context(&auth).await?;
    let keys = manifest::official_keys()?;
    let p = preview(&base, &token, &preview_id, user, &keys).await?;
    if p.binding.content_digest != content_digest {
        return Err(mismatch());
    }
    tokio::time::timeout(
        Duration::from_secs(120),
        read_check(&base, &token, &p, &keys, &request_id),
    )
    .await
    .map_err(|_| {
        error(
            "LEARN_CHECK_TIMEOUT",
            "教材联调超时，可使用原请求编号重试核对回执",
            true,
        )
    })?
}
#[cfg(test)]
mod tests;

//! Authenticated, bounded, read-only cloud course delivery. No persistent progress or execution.
use std::{collections::HashSet, sync::Arc};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use super::manifest::{self, EnrollmentSummary, ManifestResponse, TrustedKeys, VerifiedCourse};
use super::{capabilities_url, error, get_response, schema_error, Availability, LearnError};
use crate::auth::state::AuthState;

const MAX_COURSE_RESPONSE: usize = 1024 * 1024;
#[derive(Deserialize)]
struct Data<T> {
    data: T,
}
#[derive(Deserialize)]
struct Home {
    enrolled: Vec<EnrollmentSummary>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnHome {
    pub availability: Availability,
    pub enrolled: Vec<EnrollmentSummary>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSnapshot {
    pub resource_id: String,
    pub sha256: String,
    pub markdown: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnUnitContent {
    pub enrollment_id: String,
    pub release_id: String,
    pub unit_id: String,
    pub documents: Vec<DocumentSnapshot>,
}
pub(super) fn url(base: &str, segments: &[&str]) -> Result<reqwest::Url, LearnError> {
    let mut result = capabilities_url(base, cfg!(debug_assertions))?;
    result.set_query(None);
    result.set_path("/api/v2/lynx/learn/");
    if segments.iter().any(|s| !manifest::valid_id(s)) {
        return Err(schema_error());
    }
    result
        .path_segments_mut()
        .map_err(|_| schema_error())?
        .pop_if_empty()
        .extend(segments);
    Ok(result)
}
pub(super) async fn read<T: DeserializeOwned>(
    base: &str,
    token: &str,
    segments: &[&str],
) -> Result<T, LearnError> {
    let bytes = get_response(url(base, segments)?, token, MAX_COURSE_RESPONSE, false)
        .await?
        .ok_or_else(schema_error)?;
    let body: Data<T> = serde_json::from_slice(&bytes).map_err(|_| schema_error())?;
    Ok(body.data)
}
fn parse_home(bytes: &[u8]) -> Result<Vec<EnrollmentSummary>, LearnError> {
    let home: Data<Home> = serde_json::from_slice(bytes).map_err(|_| schema_error())?;
    if home.data.enrolled.len() > 256 {
        return Err(schema_error());
    }
    let mut seen = HashSet::new();
    for e in &home.data.enrolled {
        manifest::validate_enrollment(e)?;
        if !seen.insert(e.enrollment_id.as_str()) {
            return Err(schema_error());
        }
    }
    Ok(home.data.enrolled)
}
async fn home(base: &str, token: &str) -> Result<LearnHome, LearnError> {
    match get_response(url(base, &["home"])?, token, MAX_COURSE_RESPONSE, true).await? {
        None => Ok(LearnHome {
            availability: Availability::NotDeployed,
            enrolled: vec![],
        }),
        Some(bytes) => Ok(LearnHome {
            availability: Availability::Available,
            enrolled: parse_home(&bytes)?,
        }),
    }
}
pub(super) async fn context(auth: &AuthState) -> Result<(String, String), LearnError> {
    let base = std::env::var("LYNX_LEARNING_API_BASE_URL")
        .unwrap_or_else(|_| "https://www.100ask.net".into());
    url(&base, &["home"])?;
    let token = auth
        .get_access_token()
        .await
        .map_err(|_| error("LEARN_AUTH_REQUIRED", "请先登录官网账号，再读取课程", false))?;
    Ok((base, token))
}
pub(super) async fn course(
    base: &str,
    token: &str,
    enrollment_id: &str,
    release_id: &str,
    digest: &str,
    keys: &TrustedKeys,
) -> Result<VerifiedCourse, LearnError> {
    if !manifest::valid_id(enrollment_id)
        || !manifest::valid_id(release_id)
        || !manifest::valid_digest(digest)
    {
        return Err(schema_error());
    }
    let enrolled = home(base, token)
        .await?
        .enrolled
        .into_iter()
        .find(|e| e.enrollment_id == enrollment_id)
        .ok_or_else(|| {
            error(
                "LEARN_ENTITLEMENT_REQUIRED",
                "账号没有这门课程的有效报名记录，请重新同步",
                false,
            )
        })?;
    if enrolled.release_id != release_id || enrolled.manifest_sha256 != digest {
        return Err(error(
            "LEARN_RELEASE_MISMATCH",
            "课程版本发生变化，请返回课程列表重新同步",
            false,
        ));
    }
    let response: ManifestResponse =
        read(base, token, &["enrollments", enrollment_id, "manifest"]).await?;
    manifest::verify_course(response, &enrolled, keys)
}
pub fn validate_documents(
    content: &LearnUnitContent,
    course: &VerifiedCourse,
    unit_id: &str,
) -> Result<(), LearnError> {
    let unit = course
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .find(|u| u.unit_id == unit_id)
        .ok_or_else(schema_error)?;
    if content.enrollment_id != course.enrollment_id
        || content.release_id != course.manifest.release_id
        || content.unit_id != unit_id
    {
        return Err(error(
            "LEARN_UNIT_MISMATCH",
            "教材不属于当前课程章节，已阻止显示",
            false,
        ));
    }
    let resource_ids: HashSet<_> = unit
        .activities
        .iter()
        .flat_map(|a| a.resource_refs.iter())
        .collect();
    let expected: HashSet<_> = course
        .manifest
        .resources
        .iter()
        .filter(|r| r.kind == "document" && resource_ids.contains(&r.resource_id))
        .map(|r| r.resource_id.as_str())
        .collect();
    let actual: HashSet<_> = content
        .documents
        .iter()
        .map(|d| d.resource_id.as_str())
        .collect();
    if content.documents.len() > 32 || actual.len() != content.documents.len() || actual != expected
    {
        return Err(schema_error());
    }
    for d in &content.documents {
        let resource = course
            .manifest
            .resources
            .iter()
            .find(|r| r.resource_id == d.resource_id)
            .ok_or_else(schema_error)?;
        if d.markdown.len() > 512 * 1024
            || !manifest::valid_digest(&d.sha256)
            || resource.sha256.as_deref() != Some(&d.sha256)
            || resource.size != Some(d.markdown.len() as u64)
            || hex::encode(Sha256::digest(d.markdown.as_bytes())) != d.sha256
        {
            return Err(error(
                "LEARN_DOCUMENT_MISMATCH",
                "教材快照与签名版本不一致，请重新同步",
                false,
            ));
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn learn_get_home(auth: State<'_, Arc<AuthState>>) -> Result<LearnHome, LearnError> {
    let (base, token) = context(&auth).await?;
    home(&base, &token).await
}
#[tauri::command]
pub async fn learn_get_manifest(
    auth: State<'_, Arc<AuthState>>,
    enrollment_id: String,
    release_id: String,
    manifest_sha256: String,
) -> Result<VerifiedCourse, LearnError> {
    let (base, token) = context(&auth).await?;
    let keys = manifest::official_keys()?;
    course(
        &base,
        &token,
        &enrollment_id,
        &release_id,
        &manifest_sha256,
        &keys,
    )
    .await
}
#[tauri::command]
pub async fn learn_get_unit(
    auth: State<'_, Arc<AuthState>>,
    enrollment_id: String,
    release_id: String,
    manifest_sha256: String,
    unit_id: String,
) -> Result<LearnUnitContent, LearnError> {
    if !manifest::valid_id(&unit_id) {
        return Err(schema_error());
    }
    let (base, token) = context(&auth).await?;
    let keys = manifest::official_keys()?;
    let verified = course(
        &base,
        &token,
        &enrollment_id,
        &release_id,
        &manifest_sha256,
        &keys,
    )
    .await?;
    if !verified
        .manifest
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .any(|u| u.unit_id == unit_id)
    {
        return Err(schema_error());
    }
    let content: LearnUnitContent = read(
        &base,
        &token,
        &["enrollments", &enrollment_id, "units", &unit_id],
    )
    .await?;
    validate_documents(&content, &verified, &unit_id)?;
    Ok(content)
}
#[cfg(test)]
mod tests;

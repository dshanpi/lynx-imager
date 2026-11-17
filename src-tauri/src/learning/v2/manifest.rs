//! Namespaced, signed course data. This module deliberately provides no executor.
use std::collections::{HashMap, HashSet};

use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{error, schema_error, LearnError};

pub const MAX_MANIFEST_BYTES: usize = 512 * 1024;
pub const MAX_POLICY_BYTES: usize = 64 * 1024;
pub const READ_CAPABILITIES: &[&str] = &[
    "learn.capabilities.v1",
    "learn.course-manifest.v1",
    "learn.course-manifest.v2",
    "learn.course-read.v1",
    "learn.project-task.v1",
    "learn.board-observation.v1",
];

fn non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

macro_rules! dto {
    ($name:ident { $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Deserialize, Serialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name { $($(#[$attr])* pub $field: $ty),* }
    };
}
dto!(CourseManifest {
    format: String, schema_version: u32, course_id: String, release_id: String,
    version: String, title: String, track: String, required_client_capabilities: Vec<String>,
    resources: Vec<CourseResource>, environments: Vec<EnvironmentProfile>,
    actions: Vec<CourseAction>, rules: Vec<AssessmentRule>, chapters: Vec<Chapter>
});
dto!(CourseResource {
    resource_id: String, kind: String, title: String, source_url: String,
    #[serde(default, deserialize_with = "non_null_option")] sha256: Option<String>,
    #[serde(default, deserialize_with = "non_null_option")] size: Option<u64>,
    #[serde(default, deserialize_with = "non_null_option")] artifact_ref: Option<String>,
    #[serde(default, deserialize_with = "non_null_option")] commit: Option<String>
});
dto!(EnvironmentProfile {
    profile_id: String, target: String, required_capabilities: Vec<String>, version_checks: Vec<VersionCheck>
});
dto!(VersionCheck {
    component: String,
    operator: String,
    expected: String
});
dto!(CourseAction {
    action_id: String,
    #[serde(default, deserialize_with = "non_null_option", skip_serializing_if = "Option::is_none")] capability_id: Option<String>,
    catalog_version: String, target: String, risk: String,
    parameters: serde_json::Map<String, serde_json::Value>, evidence_outputs: Vec<String>
});
dto!(AssessmentRule {
    rule_id: String, validator_id: String, validator_version: String,
    parameters: serde_json::Map<String, serde_json::Value>, required_evidence_kinds: Vec<String>, minimum_trust: String
});
dto!(Chapter { chapter_id: String, title: String, units: Vec<CourseUnit>, summary_prompt: String });
dto!(CourseUnit { unit_id: String, title: String, objective: String, activities: Vec<Activity> });
dto!(Activity {
    activity_id: String, kind: String, title: String, instruction: String, expected_observation: String,
    recovery_hint: String, required: bool, resource_refs: Vec<String>, environment_ref: String,
    action_refs: Vec<String>, rule_refs: Vec<String>, completion_mode: String
});
dto!(SignedEnvelope {
    format: String,
    schema_version: u32,
    key_id: String,
    algorithm: String,
    payload_encoding: String,
    payload: String,
    sha256: String,
    signature: String
});
dto!(CoursePolicy {
    format: String,
    schema_version: u32,
    release_id: String,
    manifest_sha256: String,
    agent_policy: AgentPolicy
});
dto!(AgentPolicy {
    enabled: bool, allowed_tool_ids: Vec<String>, hint_levels: Vec<String>,
    context_resource_refs: Vec<String>, max_context_bytes: u32, allow_image_analysis: bool
});

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentSummary {
    #[serde(default)]
    pub course_slug: Option<String>,
    pub enrollment_id: String,
    pub course_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub title: String,
    pub version: String,
    pub summary: String,
    pub track: String,
    pub current_unit_id: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub state: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestResponse {
    pub content_envelope: SignedEnvelope,
    pub policy_envelope: SignedEnvelope,
    pub entitlement: Entitlement,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedCourse {
    pub enrollment_id: String,
    pub manifest_sha256: String,
    pub signing_key_id: String,
    pub manifest: CourseManifest,
    pub missing_capabilities: Vec<String>,
    pub execution_enabled: bool,
    pub agent_policy: AgentPolicy,
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
pub fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.contains('\0')
}
fn one(s: &str, options: &[&str]) -> bool {
    options.contains(&s)
}
fn ids(items: &[String]) -> bool {
    items.len() <= 512
        && items.iter().all(|s| valid_id(s))
        && items.iter().collect::<HashSet<_>>().len() == items.len()
}
fn unique<'a>(items: impl Iterator<Item = &'a str>) -> Result<HashSet<&'a str>, LearnError> {
    let mut set = HashSet::new();
    for id in items {
        if !valid_id(id) || !set.insert(id) {
            return Err(schema_error());
        }
    }
    Ok(set)
}
pub fn validate_enrollment(e: &EnrollmentSummary) -> Result<(), LearnError> {
    if ![&e.enrollment_id, &e.course_id, &e.release_id]
        .iter()
        .all(|s| valid_id(s))
        || !valid_digest(&e.manifest_sha256)
        || !text(&e.title, 512)
        || !text(&e.version, 64)
        || e.summary.len() > 8192
        || !valid_id(&e.track)
        || e.current_unit_id.as_deref().is_some_and(|s| !valid_id(s))
    {
        return Err(schema_error());
    }
    Ok(())
}

pub fn validate_manifest(m: &CourseManifest) -> Result<(), LearnError> {
    if m.format != "100ask.learn-course" || !matches!(m.schema_version, 1 | 2) {
        return Err(error(
            "LEARN_MANIFEST_UNSUPPORTED",
            "课程清单版本不受支持，请更新客户端",
            false,
        ));
    }
    if !valid_id(&m.course_id)
        || !valid_id(&m.release_id)
        || !text(&m.title, 512)
        || !regex::Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9.-]+)?$")
            .expect("fixed regex")
            .is_match(&m.version)
        || m.version.len() > 64
        || !one(
            &m.track,
            &["kvm", "multimedia", "bsp", "linux_app", "theory", "custom"],
        )
        || m.required_client_capabilities.is_empty()
        || !ids(&m.required_client_capabilities)
        || m.resources.len() > 512
        || m.environments.is_empty()
        || m.environments.len() > 128
        || m.actions.len() > 512
        || m.rules.len() > 512
        || m.chapters.is_empty()
        || m.chapters.len() > 512
    {
        return Err(schema_error());
    }
    if m.schema_version == 2
        && !m
            .required_client_capabilities
            .iter()
            .any(|c| c == "learn.course-manifest.v2")
    {
        return Err(schema_error());
    }
    let resources = unique(m.resources.iter().map(|r| r.resource_id.as_str()))?;
    let environments = unique(m.environments.iter().map(|e| e.profile_id.as_str()))?;
    let actions = unique(m.actions.iter().map(|a| a.action_id.as_str()))?;
    let rules = unique(m.rules.iter().map(|r| r.rule_id.as_str()))?;
    unique(m.chapters.iter().map(|c| c.chapter_id.as_str()))?;
    unique(
        m.chapters
            .iter()
            .flat_map(|c| c.units.iter().map(|u| u.unit_id.as_str())),
    )?;
    unique(
        m.chapters
            .iter()
            .flat_map(|c| &c.units)
            .flat_map(|u| u.activities.iter().map(|a| a.activity_id.as_str())),
    )?;
    let unit_count: usize = m.chapters.iter().map(|c| c.units.len()).sum();
    let activity_count: usize = m
        .chapters
        .iter()
        .flat_map(|c| &c.units)
        .map(|u| u.activities.len())
        .sum();
    if unit_count > 512 || activity_count > 2048 {
        return Err(schema_error());
    }
    for r in &m.resources {
        let url = reqwest::Url::parse(&r.source_url).map_err(|_| schema_error())?;
        if !text(&r.title, 512)
            || !one(
                &r.kind,
                &["document", "slides", "video", "source", "sdk", "image"],
            )
            || r.source_url.len() > 2048
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || r.sha256.as_deref().is_some_and(|s| !valid_digest(s))
            || r.artifact_ref.as_deref().is_some_and(|s| !valid_id(s))
            || (r.kind != "video"
                && (r.sha256.is_none() || r.size.is_none() || r.artifact_ref.is_none()))
            || r.commit.as_deref().is_some_and(|s| {
                !matches!(s.len(), 40 | 64)
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            || (r.kind == "source" && r.commit.is_none())
        {
            return Err(schema_error());
        }
    }
    for e in &m.environments {
        if !one(&e.target, &["none", "ubuntu", "board"])
            || !ids(&e.required_capabilities)
            || e.version_checks.len() > 32
        {
            return Err(schema_error());
        }
        for v in &e.version_checks {
            if !one(
                &v.component,
                &["os", "toolchain", "sdk", "image", "runtime"],
            ) || !one(&v.operator, &["eq", "semver_range", "sha256"])
                || !text(&v.expected, 256)
                || (v.operator == "sha256" && !valid_digest(&v.expected))
            {
                return Err(schema_error());
            }
        }
    }
    for a in &m.actions {
        if (m.schema_version == 1 && a.capability_id.is_some())
            || (m.schema_version == 2 && !a.capability_id.as_deref().is_some_and(valid_id))
        {
            return Err(schema_error());
        }
        if !text(&a.catalog_version, 64)
            || !one(&a.target, &["ubuntu", "board"])
            || !one(
                &a.risk,
                &[
                    "read_only",
                    "workspace_write",
                    "device_write",
                    "destructive",
                ],
            )
            || a.evidence_outputs.is_empty()
            || !ids(&a.evidence_outputs)
        {
            return Err(schema_error());
        }
    }
    for r in &m.rules {
        if !valid_id(&r.validator_id)
            || !text(&r.validator_version, 64)
            || r.required_evidence_kinds.is_empty()
            || !ids(&r.required_evidence_kinds)
            || !one(
                &r.minimum_trust,
                &["user_asserted", "client_measured", "server_verified"],
            )
        {
            return Err(schema_error());
        }
    }
    for c in &m.chapters {
        if !text(&c.title, 512) || !text(&c.summary_prompt, 8192) || c.units.is_empty() {
            return Err(schema_error());
        }
        for u in &c.units {
            if !text(&u.title, 512) || !text(&u.objective, 8192) || u.activities.is_empty() {
                return Err(schema_error());
            }
            for a in &u.activities {
                if !text(&a.title, 512)
                    || ![&a.instruction, &a.expected_observation, &a.recovery_hint]
                        .iter()
                        .all(|s| text(s, 16384))
                    || !one(
                        &a.kind,
                        &[
                            "read",
                            "understand",
                            "code",
                            "build",
                            "deploy",
                            "verify",
                            "reflect",
                        ],
                    )
                    || !one(&a.completion_mode, &["user_confirmed", "quiz", "validator"])
                    || !environments.contains(a.environment_ref.as_str())
                    || ![&a.resource_refs, &a.action_refs, &a.rule_refs]
                        .iter()
                        .all(|v| ids(v))
                    || !a
                        .resource_refs
                        .iter()
                        .all(|id| resources.contains(id.as_str()))
                    || !a.action_refs.iter().all(|id| actions.contains(id.as_str()))
                    || !a.rule_refs.iter().all(|id| rules.contains(id.as_str()))
                    || (a.completion_mode == "validator" && a.rule_refs.is_empty())
                {
                    return Err(schema_error());
                }
                let env = m
                    .environments
                    .iter()
                    .find(|e| e.profile_id == a.environment_ref)
                    .ok_or_else(schema_error)?;
                for id in &a.action_refs {
                    if m.actions
                        .iter()
                        .find(|x| &x.action_id == id)
                        .is_none_or(|x| x.target != env.target)
                    {
                        return Err(schema_error());
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedKey {
    key_id: String,
    public_key: String,
}
pub type TrustedKeys = HashMap<String, VerifyingKey>;
pub fn parse_trusted_keys(json: &str) -> Result<TrustedKeys, LearnError> {
    let invalid = || {
        error(
            "LEARN_SIGNER_CONFIG_INVALID",
            "课程官方公钥配置无效，请联系发布管理员",
            false,
        )
    };
    if json.len() > 8192 {
        return Err(invalid());
    }
    let entries: Vec<TrustedKey> = serde_json::from_str(json).map_err(|_| invalid())?;
    if entries.is_empty() || entries.len() > 16 {
        return Err(invalid());
    }
    let mut keys = HashMap::new();
    for entry in entries {
        if !valid_id(&entry.key_id) {
            return Err(invalid());
        }
        let bytes: [u8; 32] = STANDARD
            .decode(&entry.public_key)
            .map_err(|_| invalid())?
            .try_into()
            .map_err(|_| invalid())?;
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| invalid())?;
        if key.is_weak() || keys.insert(entry.key_id, key).is_some() {
            return Err(invalid());
        }
    }
    Ok(keys)
}
pub fn official_keys() -> Result<TrustedKeys, LearnError> {
    let raw = option_env!("LYNX_LEARN_TRUSTED_KEYS_JSON")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(include_str!("official-public-keys.json"));
    parse_trusted_keys(raw)
}
pub(super) fn verify<T: DeserializeOwned>(
    envelope: &SignedEnvelope,
    keys: &TrustedKeys,
    limit: usize,
) -> Result<T, LearnError> {
    if envelope.format != "100ask.learn-envelope"
        || envelope.schema_version != 1
        || envelope.algorithm != "Ed25519-SHA256"
        || envelope.payload_encoding != "base64"
        || !valid_digest(&envelope.sha256)
        || envelope.payload.len() > limit.div_ceil(3) * 4
        || envelope.signature.len() != 88
    {
        return Err(schema_error());
    }
    let key = keys.get(&envelope.key_id).ok_or_else(|| {
        error(
            "LEARN_SIGNER_UNTRUSTED",
            "课程签名来源不受信任，已阻止打开",
            false,
        )
    })?;
    let bytes = STANDARD
        .decode(&envelope.payload)
        .map_err(|_| schema_error())?;
    if bytes.len() > limit {
        return Err(schema_error());
    }
    let digest = Sha256::digest(&bytes);
    if hex::encode(digest) != envelope.sha256 {
        return Err(error(
            "LEARN_DIGEST_MISMATCH",
            "课程内容摘要不匹配，已阻止打开",
            false,
        ));
    }
    let signature = STANDARD
        .decode(&envelope.signature)
        .map_err(|_| schema_error())?;
    let signature = Signature::from_slice(&signature).map_err(|_| schema_error())?;
    key.verify_strict(&digest, &signature).map_err(|_| {
        error(
            "LEARN_SIGNATURE_INVALID",
            "课程签名验证失败，已阻止打开",
            false,
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|_| schema_error())
}
pub fn verify_course(
    response: ManifestResponse,
    enrollment: &EnrollmentSummary,
    keys: &TrustedKeys,
) -> Result<VerifiedCourse, LearnError> {
    if response.entitlement.state != "active" {
        return Err(error(
            "LEARN_ENTITLEMENT_REQUIRED",
            "课程权益已失效或尚未开通，请重新同步",
            false,
        ));
    }
    let manifest: CourseManifest = verify(&response.content_envelope, keys, MAX_MANIFEST_BYTES)?;
    let policy: CoursePolicy = verify(&response.policy_envelope, keys, MAX_POLICY_BYTES)?;
    validate_manifest(&manifest)?;
    if manifest.course_id != enrollment.course_id
        || manifest.release_id != enrollment.release_id
        || manifest.version != enrollment.version
        || response.content_envelope.sha256 != enrollment.manifest_sha256
        || policy.format != "100ask.learn-policy"
        || policy.schema_version != 1
        || policy.release_id != manifest.release_id
        || policy.manifest_sha256 != enrollment.manifest_sha256
    {
        return Err(error(
            "LEARN_RELEASE_MISMATCH",
            "课程发布版本发生变化，请返回课程列表重新同步",
            false,
        ));
    }
    let agent = policy.agent_policy;
    if !ids(&agent.allowed_tool_ids)
        || !ids(&agent.context_resource_refs)
        || agent.max_context_bytes > 262144
        || agent.hint_levels.len() > 4
        || !ids(&agent.hint_levels)
        || !agent
            .hint_levels
            .iter()
            .all(|h| one(h, &["explain", "hint", "example", "solution"]))
        || !agent
            .context_resource_refs
            .iter()
            .all(|id| manifest.resources.iter().any(|r| &r.resource_id == id))
    {
        return Err(schema_error());
    }
    let missing_capabilities: Vec<String> = manifest
        .required_client_capabilities
        .iter()
        .filter(|c| !READ_CAPABILITIES.contains(&c.as_str()))
        .cloned()
        .collect();
    let execution_enabled = missing_capabilities.is_empty()
        && manifest.schema_version == 2
        && !manifest.actions.is_empty()
        && manifest.actions.iter().all(|a| {
            (a.capability_id.as_deref() == Some("learn.project.task")
                && a.catalog_version == "1.0.0"
                && a.target == "ubuntu"
                && a.risk == "workspace_write")
                || super::board::supported(a)
        });
    Ok(VerifiedCourse {
        enrollment_id: enrollment.enrollment_id.clone(),
        manifest_sha256: enrollment.manifest_sha256.clone(),
        signing_key_id: response.content_envelope.key_id,
        manifest,
        missing_capabilities,
        execution_enabled,
        agent_policy: agent,
    })
}

#[cfg(test)]
pub(super) mod tests;

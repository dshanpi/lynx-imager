//! Execution-bound observations of signed requirements; not device evidence.
use super::*;

pub(super) const CHANGED: &str =
    "LEARNING_INPUTS_CHANGED:课程声明的 SDK 或系统工具环境不符或已变化，请处理差异后重新实践";
pub(super) const UNAVAILABLE: &str =
    "LEARNING_INPUTS_UNAVAILABLE:无法核验课程声明的环境，请检查连接、依赖与读取预算";
pub(super) const LEGACY: &str =
    "LEARNING_INPUTS_UNRECORDED:旧执行未记录本活动要求的环境，请重新实践";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Fact {
    id: String,
    kind: String,
    value: String,
    tool_sha256: String,
    path_sha256: String,
    tool_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Receipt {
    version: u32,
    requirements_sha256: String,
    identity_sha256: String,
    observations: Vec<Fact>,
}

fn hash(value: &impl Serialize) -> Result<String, String> {
    Ok(sha256(&serde_json::to_vec(value).map_err(|_| UNAVAILABLE)?))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn display(s: &str) -> String {
    let clean = crate::supervisor::redact_learning_context(s);
    let mut value = String::new();
    for ch in clean.chars().filter(|c| !c.is_control()) {
        if value.len() + ch.len_utf8() > 128 {
            break;
        }
        value.push(ch);
    }
    value
}

impl Receipt {
    pub(super) fn valid(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        self.version == 1
            && valid_hash(&self.requirements_sha256)
            && valid_hash(&self.identity_sha256)
            && !self.observations.is_empty()
            && self.observations.len() <= 16
            && self.observations.iter().all(|f| {
                valid_id(&f.id)
                    && ids.insert(&f.id)
                    && matches!(
                        f.kind.as_str(),
                        "tool" | "file" | "os" | "architecture" | "git"
                    )
                    && f.value.len() <= 128
                    && f.tool_version.len() <= 128
                    && !f.value.chars().any(char::is_control)
                    && !f.tool_version.chars().any(char::is_control)
                    && if f.kind == "tool" {
                        valid_hash(&f.tool_sha256) && valid_hash(&f.path_sha256)
                    } else {
                        f.tool_sha256.is_empty()
                            && f.path_sha256.is_empty()
                            && f.tool_version.is_empty()
                    }
            })
            && serde_json::to_vec(self).is_ok_and(|v| v.len() <= 16000)
    }
}

pub(super) fn require(
    receipt: &Option<Receipt>,
    specs: &[environment::CheckSpec],
) -> Result<(), String> {
    match receipt {
        None if specs.is_empty() => Ok(()),
        None => Err(LEGACY.into()),
        Some(r)
            if r.valid()
                && r.requirements_sha256 == hash(&specs)?
                && r.observations.len() == specs.len()
                && r.observations
                    .iter()
                    .zip(specs)
                    .all(|(f, s)| f.id == s.id && f.kind == s.kind) =>
        {
            Ok(())
        }
        Some(_) => Err(UNAVAILABLE.into()),
    }
}

pub(super) fn seal(bytes: &[u8], specs: &[environment::CheckSpec]) -> Result<Receipt, String> {
    // decode re-evaluates expected values before any display redaction.
    let checked = environment::decode(bytes, specs).map_err(|_| UNAVAILABLE)?;
    if checked.checks.iter().any(|c| c.status == "UNVERIFIED") {
        return Err(UNAVAILABLE.into());
    }
    if checked.checks.iter().any(|c| c.status != "MATCH") {
        return Err(CHANGED.into());
    }
    let raw: environment::Observation = serde_json::from_slice(bytes).map_err(|_| UNAVAILABLE)?;
    let mut identities = Vec::new();
    let mut observations = Vec::new();
    for check in &raw.checks {
        let tool = if check.spec.kind == "tool" {
            Some(
                raw.tools
                    .iter()
                    .find(|t| t.name == check.spec.selector && t.status == "OBSERVED")
                    .ok_or(UNAVAILABLE)?,
            )
        } else {
            None
        };
        identities.push(serde_json::json!({"id":check.spec.id,"value":check.observed,
            "tool":tool.map(|t|serde_json::json!({"name":t.name,"path":t.path,"version":t.version,"sha256":t.sha256}))}));
        observations.push(Fact {
            id: check.spec.id.clone(),
            kind: check.spec.kind.clone(),
            value: display(&check.observed),
            tool_sha256: tool.map(|t| t.sha256.clone()).unwrap_or_default(),
            path_sha256: tool.map(|t| sha256(t.path.as_bytes())).unwrap_or_default(),
            tool_version: tool.map(|t| display(&t.version)).unwrap_or_default(),
        });
    }
    let receipt = Receipt {
        version: 1,
        requirements_sha256: hash(&specs)?,
        identity_sha256: hash(&identities)?,
        observations,
    };
    if !receipt.valid() {
        return Err(UNAVAILABLE.into());
    }
    Ok(receipt)
}

pub(super) fn checked(
    ssh: &ssh2::Session,
    sdk: &str,
    specs: &[environment::CheckSpec],
) -> Result<Option<Receipt>, String> {
    if specs.is_empty() {
        return Ok(None);
    }
    let (_, bytes) = environment::probe(ssh, sdk, specs).map_err(|_| UNAVAILABLE)?;
    seal(&bytes, specs).map(Some)
}

pub(super) fn compare(
    expected: &Option<Receipt>,
    current: &Option<Receipt>,
    specs: &[environment::CheckSpec],
) -> Result<(), String> {
    require(expected, specs)?;
    require(current, specs)?;
    if expected != current {
        return Err(CHANGED.into());
    }
    Ok(())
}

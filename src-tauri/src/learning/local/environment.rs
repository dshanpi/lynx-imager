//! Current, read-only host/project observations, never course completion evidence.
use super::*;
use crate::workbench::WorkbenchState;

pub(super) const TOOLS: &[&str] = &[
    "cc",
    "gcc",
    "make",
    "cmake",
    "git",
    "python3",
    "dtc",
    "arm-linux-gnueabihf-gcc",
    "aarch64-linux-gnu-gcc",
    "ffmpeg",
    "ffprobe",
    "pkg-config",
];
const PROBE: &str = include_str!("environment_probe.py");
const INVALID: &str = "LOCAL_COURSE_ENVIRONMENT_INVALID";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckSpec {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub selector: String,
    pub operator: String,
    pub expected: String,
    pub remediation: String,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}
pub(super) fn validate(manifest: &LocalCourseManifest, lab: &LocalLab) -> Result<(), String> {
    let specs = &lab.environment_checks;
    if specs.is_empty() {
        return Ok(());
    }
    if manifest.schema_version < 7 || theory::is_reflection(lab) || specs.len() > 16 {
        return Err(INVALID.into());
    }
    let mut ids = std::collections::HashSet::new();
    for spec in specs {
        let valid = match (spec.kind.as_str(), spec.operator.as_str()) {
            ("tool", "present") => {
                TOOLS.contains(&spec.selector.as_str()) && spec.expected.is_empty()
            }
            ("tool", "eq") => {
                TOOLS.contains(&spec.selector.as_str()) && !spec.expected.trim().is_empty()
            }
            ("tool", "sha256") => TOOLS.contains(&spec.selector.as_str()) && digest(&spec.expected),
            ("file", "sha256") => {
                safe_relative(&spec.selector).is_ok()
                    && !spec.selector.contains('\\')
                    && digest(&spec.expected)
            }
            ("os" | "architecture", "eq") => {
                spec.selector.is_empty() && !spec.expected.trim().is_empty()
            }
            ("git", "eq") => {
                spec.selector.is_empty()
                    && [40, 64].contains(&spec.expected.len())
                    && spec
                        .expected
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
            _ => false,
        };
        if !valid
            || !valid_id(&spec.id)
            || !ids.insert(&spec.id)
            || spec.title.trim().is_empty()
            || spec.remediation.trim().is_empty()
            || !text(&spec.title, 256)
            || !text(&spec.selector, 1024)
            || !text(&spec.expected, 512)
            || !text(&spec.remediation, 1024)
        {
            return Err(INVALID.into());
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub kind: String,
    pub commit: String,
    pub branch: String,
    pub dirty: Option<bool>,
    pub untracked: Option<bool>,
    pub status: String,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Host {
    pub os: String,
    pub architecture: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub status: String,
    pub path: String,
    pub version: String,
    pub sha256: String,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub spec: CheckSpec,
    pub observed: String,
    pub status: String,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub source: Source,
    pub host: Host,
    pub tools: Vec<Tool>,
    pub checks: Vec<CheckResult>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub output: String,
    pub checked_at: i64,
    pub project_id: i64,
    pub step_id: String,
    pub attempt: i64,
    pub fingerprint: String,
    #[serde(flatten)]
    pub observation: Observation,
}

pub(super) fn decode(bytes: &[u8], specs: &[CheckSpec]) -> Result<Observation, String> {
    let bad = "LEARN_INSPECT_RESPONSE_INVALID";
    if bytes.len() > 65_536 {
        return Err(bad.into());
    }
    let mut value: Observation = serde_json::from_slice(bytes).map_err(|_| bad)?;
    if value.checks.len() != specs.len()
        || value.tools.len() > TOOLS.len()
        || !matches!(
            value.source.kind.as_str(),
            "git" | "unborn" | "directory" | "unavailable"
        )
        || (value.source.kind == "git"
            && (!matches!(value.source.commit.len(), 40 | 64)
                || !value.source.commit.bytes().all(|b| b.is_ascii_hexdigit())))
        || value.checks.iter().zip(specs).any(|(check, spec)| {
            &check.spec != spec
                || !matches!(
                    check.status.as_str(),
                    "MATCH" | "MISMATCH" | "MISSING" | "UNVERIFIED"
                )
        })
        || value.tools.iter().any(|tool| {
            !TOOLS.contains(&tool.name.as_str())
                || !matches!(tool.status.as_str(), "OBSERVED" | "MISSING" | "UNVERIFIED")
                || (tool.status == "OBSERVED"
                    && (!digest(&tool.sha256)
                        || tool.version.is_empty()
                        || !tool.path.starts_with('/')))
        })
    {
        return Err(bad.into());
    }
    let mut tool_names = std::collections::HashSet::new();
    if value
        .tools
        .iter()
        .any(|tool| !tool_names.insert(&tool.name))
    {
        return Err(bad.into());
    }
    for check in &mut value.checks {
        if matches!(check.status.as_str(), "MATCH" | "MISMATCH") {
            if check.observed.is_empty() {
                return Err(bad.into());
            }
            let actual = match check.spec.kind.as_str() {
                "tool" => {
                    let tool = value
                        .tools
                        .iter()
                        .find(|tool| tool.name == check.spec.selector && tool.status == "OBSERVED")
                        .ok_or(bad)?;
                    if check.spec.operator == "sha256" {
                        &tool.sha256
                    } else {
                        &tool.version
                    }
                }
                "os" => &value.host.os,
                "architecture" => &value.host.architecture,
                "git" if value.source.kind == "git" => &value.source.commit,
                "file" if digest(&check.observed) => &check.observed,
                _ => return Err(bad.into()),
            };
            if actual != &check.observed {
                return Err(bad.into());
            }
            // The client computes comparisons; a status string is not an authority.
            check.status =
                if check.spec.operator == "present" || check.observed == check.spec.expected {
                    "MATCH"
                } else {
                    "MISMATCH"
                }
                .into();
        }
    }
    // Redact observations before returning them to UI/tutor consumers. No raw output is persisted.
    let clean = crate::supervisor::redact_learning_context;
    value.source.branch = clean(&value.source.branch);
    value.source.status = clean(&value.source.status);
    value.host.os = clean(&value.host.os);
    for tool in &mut value.tools {
        tool.version = clean(&tool.version);
        tool.path = clean(&tool.path);
    }
    for check in &mut value.checks {
        check.observed = clean(&check.observed);
    }
    Ok(value)
}

pub(super) fn probe(
    ssh: &ssh2::Session,
    sdk: &str,
    specs: &[CheckSpec],
) -> Result<(Observation, Vec<u8>), String> {
    let encoded = serde_json::to_string(&specs).map_err(|_| INVALID)?;
    ssh.set_timeout(20_000);
    let mut channel = ssh.channel_session().map_err(|_| "LEARN_INSPECT_CHANNEL")?;
    let command = format!(
        "cd {} && /usr/bin/python3 -I -S -c {} {}",
        crate::workbench::ssh::shell_quote(sdk),
        crate::workbench::ssh::shell_quote(PROBE),
        crate::workbench::ssh::shell_quote(&encoded)
    );
    channel.exec(&command).map_err(|_| "LEARN_INSPECT_EXEC")?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut channel)
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "LEARN_INSPECT_READ")?;
    if bytes.len() > 65_536 {
        return Err("LEARN_INSPECT_TOO_LARGE".into());
    }
    channel.wait_close().map_err(|_| "LEARN_INSPECT_CLOSE")?;
    if channel.exit_status().map_err(|_| "LEARN_INSPECT_STATUS")? != 0 {
        return Err(
            "LEARN_INSPECT_FAILED:请核对 Ubuntu 工程目录、系统 Python 3、读取权限与检查预算".into(),
        );
    }
    let observation = decode(&bytes, specs)?;
    Ok((observation, bytes))
}

pub(super) fn ensure_context(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    session: &LocalLearningSession,
    attempt: i64,
    binding: &str,
) -> Result<(), String> {
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    ensure_action_context(&db, session, attempt)?;
    let current = get_session(&db, &session.session_id)?;
    let work = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    if current.project_id != session.project_id
        || freshness::current_binding(&work, &current)? != binding
    {
        return Err("LEARNING_INPUTS_CHANGED:工程绑定已变化，请重新检查".into());
    }
    Ok(())
}

pub(super) async fn inspect(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    session_id: &str,
) -> Result<Snapshot, String> {
    let (session, attempt, specs, project, host, binding, lease) = {
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        let session = get_session(&db, session_id)?;
        let (_, lab) = manifest_and_lab(&db, &session)?;
        if theory::is_reflection(&lab) {
            return Err("LEARNING_ACTIVITY_HAS_NO_EXECUTION".into());
        }
        let attempt = current_attempt(&db, session_id, &session.current_step)?;
        let work = workbench
            .db
            .lock()
            .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
        let (project, host) = freshness::project(&work, &session)?;
        if !project.sdk_dir.starts_with('/')
            || project.sdk_dir.len() > 4096
            || project.sdk_dir.chars().any(char::is_control)
            || project.sdk_dir.split('/').any(|p| p == "..")
        {
            return Err("LEARN_PROJECT_PATH_INVALID".into());
        }
        let binding = freshness::binding(&project, &host)?;
        let lease = local.actions.acquire(session.project_id)?;
        (
            session,
            attempt,
            lab.environment_checks,
            project,
            host,
            binding,
            lease,
        )
    };
    let credentials = crate::workbench::credentials::resolve(host.id, Default::default())?;
    let worker_specs = specs.clone();
    let (observation, bytes, _lease) =
        tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
            let (ssh, _, _) = crate::workbench::ssh::connect(&host, &credentials)?;
            let (observation, bytes) = probe(&ssh, &project.sdk_dir, &worker_specs)?;
            Ok((observation, bytes, lease))
        })
        .await
        .map_err(|_| "LEARN_INSPECT_TASK_FAILED")??;
    ensure_context(local, workbench, &session, attempt, &binding)?;
    let output = format!(
        "{}\n{}\n{}\n{}",
        observation.source.kind,
        observation.source.commit,
        observation.source.branch,
        observation.source.status
    );
    Ok(Snapshot {
        output,
        checked_at: now(),
        project_id: session.project_id,
        step_id: session.current_step,
        attempt,
        fingerprint: sha256(&[binding.as_bytes(), &bytes].concat()),
        observation,
    })
}

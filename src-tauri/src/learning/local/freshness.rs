//! Read-only course workspace observations; historical evidence is immutable.
use super::*;
use crate::workbench::types::{HostConfig, ProjectConfig, SshCredentials};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const UNAVAILABLE: &str =
    "LEARNING_INPUTS_UNAVAILABLE:无法核验当前课程工程，请检查 SSH、文件和读取预算后重试";
const CHANGED: &str = "LEARNING_INPUTS_CHANGED:课程工程已变化，请恢复原输入或重新实践";
const LEGACY: &str = "LEARNING_INPUTS_UNRECORDED:旧记录没有工程快照，请重新实践";
const MAX_ENTRIES: usize = 10000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Snapshot {
    version: u32,
    pub(super) binding_sha256: String,
    root_sha256: String,
    tree_sha256: String,
    entries: usize,
    bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) environment: Option<environment_receipt::Receipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) inputs_sha256: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct FileFact {
    sha256: String,
    bytes: u64,
    mode: u32,
}
pub(super) struct Tree {
    root: String,
    files: BTreeMap<String, FileFact>,
    bytes: u64,
}
fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    Ok(sha256(&serde_json::to_vec(value).map_err(|_| UNAVAILABLE)?))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
}
impl Snapshot {
    fn valid(&self) -> bool {
        ((self.version == 1 && self.environment.is_none() && self.inputs_sha256.is_none())
            || (self.version == 2
                && self.environment.as_ref().is_some_and(|v| v.valid())
                && self.inputs_sha256.is_none())
            || (self.version == 3
                && self.inputs_sha256.as_ref().is_some_and(|v| valid_digest(v))
                && self.environment.as_ref().is_none_or(|v| v.valid())))
            && self.entries > 0
            && self.entries <= MAX_ENTRIES
            && self.bytes <= MAX_BYTES
            && [&self.binding_sha256, &self.root_sha256, &self.tree_sha256]
                .iter()
                .all(|v| valid_digest(v))
    }
}
pub(super) fn freeze(tree: &Tree, binding: &str) -> Result<Snapshot, String> {
    let snapshot = Snapshot {
        version: 1,
        binding_sha256: binding.into(),
        root_sha256: sha256(tree.root.as_bytes()),
        tree_sha256: digest(&tree.files)?,
        entries: tree.files.len(),
        bytes: tree.bytes,
        environment: None,
        inputs_sha256: None,
    };
    if !snapshot.valid() {
        return Err(UNAVAILABLE.into());
    }
    Ok(snapshot)
}
pub(super) fn freeze_environment(
    tree: &Tree,
    binding: &str,
    environment: Option<environment_receipt::Receipt>,
) -> Result<Snapshot, String> {
    let mut snapshot = freeze(tree, binding)?;
    if environment.is_some() {
        snapshot.version = 2;
    }
    snapshot.environment = environment;
    if !snapshot.valid() {
        return Err(UNAVAILABLE.into());
    }
    Ok(snapshot)
}
pub(super) fn compare_workspace(expected: &Snapshot, current: &Snapshot) -> Result<(), String> {
    if !expected.valid() || !current.valid() {
        return Err(UNAVAILABLE.into());
    }
    let mut a = expected.clone();
    let mut b = current.clone();
    a.version = 1;
    b.version = 1;
    a.environment = None;
    b.environment = None;
    a.inputs_sha256 = None;
    b.inputs_sha256 = None;
    compare(&a, &b)
}
pub(super) fn compare(expected: &Snapshot, current: &Snapshot) -> Result<(), String> {
    if !expected.valid() || !current.valid() {
        return Err(UNAVAILABLE.into());
    }
    if expected != current {
        return Err(CHANGED.into());
    }
    Ok(())
}
pub(super) fn binding(project: &ProjectConfig, host: &HostConfig) -> Result<String, String> {
    // Configuration types contain references to credentials, never their values.
    digest(&(project, host))
}
pub(super) fn project(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(ProjectConfig, HostConfig), String> {
    let project = crate::workbench::storage::project_by_id(db, session.project_id)
        .map_err(|_| UNAVAILABLE)?;
    if project.device_id != session.device_id {
        return Err(CHANGED.into());
    }
    if let Some(id) = project.device_id {
        let device = crate::workbench::storage::device_by_id(db, id).map_err(|_| UNAVAILABLE)?;
        if device.board_uid != session.board_uid {
            return Err(CHANGED.into());
        }
    }
    let host =
        crate::workbench::storage::host_by_id(db, project.host_id).map_err(|_| UNAVAILABLE)?;
    Ok((project, host))
}
pub(super) fn current_binding(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<String, String> {
    let (project, host) = project(db, session)?;
    binding(&project, &host)
}
pub(super) fn inputs(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
) -> Result<Vec<String>, String> {
    let install: String = db
        .query_row(
            "SELECT install_path FROM local_courses WHERE course_id=?1 AND version=?2",
            params![session.course_id, session.course_version],
            |r| r.get(0),
        )
        .map_err(|_| UNAVAILABLE)?;
    let path = Path::new(&install).join("provenance/provenance.lock.json");
    let mut file = fs::File::open(path).map_err(|_| UNAVAILABLE)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() > 1024 * 1024 || sha256(&bytes) != manifest.provenance_sha256 {
        return Err(UNAVAILABLE.into());
    }
    let provenance: ProvenanceLock = serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE)?;
    if provenance.schema_version != 1
        || provenance.source_files.is_empty()
        || provenance.source_files.len() > MAX_ENTRIES
    {
        return Err(UNAVAILABLE.into());
    }
    let mut paths = Vec::new();
    for file in provenance.source_files {
        safe_relative(&file.path).map_err(|_| UNAVAILABLE)?;
        if paths.contains(&file.path) {
            return Err(UNAVAILABLE.into());
        }
        paths.push(file.path);
    }
    Ok(paths)
}
pub(super) fn unchanged_inputs(
    before: &Tree,
    after: &Tree,
    inputs: &[String],
) -> Result<(), String> {
    if before.root != after.root {
        return Err(CHANGED.into());
    }
    for path in inputs {
        let a = before.files.get(path).ok_or(UNAVAILABLE)?;
        let b = after.files.get(path).ok_or(CHANGED)?;
        if a != b {
            return Err(CHANGED.into());
        }
    }
    Ok(())
}

pub(super) fn source_identity(tree: &Tree, paths: &[String]) -> Result<String, String> {
    if paths.is_empty() || paths.len() > MAX_ENTRIES {
        return Err(UNAVAILABLE.into());
    }
    let mut selected = BTreeMap::new();
    for path in paths {
        safe_relative(path).map_err(|_| UNAVAILABLE)?;
        let fact = tree.files.get(path).ok_or(CHANGED)?;
        if fact.sha256.is_empty() {
            return Err(CHANGED.into());
        }
        if selected.insert(path, fact).is_some() {
            return Err(UNAVAILABLE.into());
        }
    }
    digest(&(&tree.root, selected))
}
pub(super) fn freeze_sources(
    tree: &Tree,
    bound: &str,
    environment: Option<environment_receipt::Receipt>,
    paths: &[String],
) -> Result<Snapshot, String> {
    let mut snapshot = freeze_environment(tree, bound, environment)?;
    snapshot.version = 3;
    snapshot.inputs_sha256 = Some(source_identity(tree, paths)?);
    if !snapshot.valid() {
        return Err(UNAVAILABLE.into());
    }
    Ok(snapshot)
}

fn refreeze(
    expected: &Snapshot,
    tree: &Tree,
    bound: &str,
    environment: Option<environment_receipt::Receipt>,
    paths: Option<&[String]>,
) -> Result<Snapshot, String> {
    // A successfully observed deletion is a change, not an observation failure.
    compare_workspace(expected, &freeze(tree, bound)?)?;
    match paths {
        Some(paths) => freeze_sources(tree, bound, environment, paths),
        None => freeze_environment(tree, bound, environment),
    }
}

// Remote paths remain POSIX even when the desktop host is Windows.
fn remote_path(root: &str, relative: &str) -> Result<PathBuf, String> {
    if !root.starts_with('/') || root.contains('\\') || root.chars().any(char::is_control) {
        return Err(UNAVAILABLE.into());
    }
    if !relative.is_empty() {
        safe_relative(relative).map_err(|_| UNAVAILABLE)?;
    }
    Ok(PathBuf::from(if relative.is_empty() {
        root.to_owned()
    } else {
        format!("{}/{relative}", root.trim_end_matches('/'))
    }))
}

fn children(
    sftp: &ssh2::Sftp,
    path: &Path,
    budget: &impl Fn() -> Result<(), String>,
) -> Result<Vec<PathBuf>, String> {
    budget()?;
    let mut dir = sftp.opendir(path).map_err(|_| UNAVAILABLE)?;
    let mut result = Vec::new();
    loop {
        budget()?;
        match dir.readdir() {
            Ok((name, _)) => {
                if name == Path::new(".") || name == Path::new("..") {
                    continue;
                }
                let text = name.to_str().ok_or(UNAVAILABLE)?;
                if name.components().count() != 1
                    || text.chars().any(char::is_control)
                    || result.len() >= MAX_ENTRIES
                {
                    return Err(UNAVAILABLE.into());
                }
                safe_relative(text).map_err(|_| UNAVAILABLE)?;
                result.push(remote_path(path.to_str().ok_or(UNAVAILABLE)?, text)?);
            }
            // ssh2 0.9.6 File::readdir uses LIBSSH2_ERROR_FILE for EOF.
            Err(error) if error.code() == ssh2::ErrorCode::Session(-16) => break,
            Err(_) => return Err(UNAVAILABLE.into()),
        }
    }
    Ok(result)
}

/// SFTP only. No shell interpolation, source upload, or remote helper dependency.
/// Unsupported links/special files and exhausted budgets fail closed.
pub(super) fn observe(ssh: &ssh2::Session, root: &str) -> Result<Tree, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let budget = || -> Result<(), String> {
        ssh.set_timeout(
            execution::remaining_timeout(deadline, Instant::now()).map_err(|_| UNAVAILABLE)?,
        );
        Ok(())
    };
    budget()?;
    let sftp = ssh.sftp().map_err(|_| UNAVAILABLE)?;
    budget()?;
    let canonical = sftp.realpath(Path::new(root)).map_err(|_| UNAVAILABLE)?;
    let canonical = canonical.to_str().ok_or(UNAVAILABLE)?.to_owned();
    if !canonical.starts_with('/') {
        return Err(UNAVAILABLE.into());
    }
    let mut tree = Tree {
        root: canonical.clone(),
        files: BTreeMap::new(),
        bytes: 0,
    };
    let mut pending = vec![(PathBuf::from(&canonical), String::new())];
    let mut buffer = [0u8; 65536];
    while let Some((path, relative)) = pending.pop() {
        budget()?;
        let stat = sftp.lstat(&path).map_err(|_| UNAVAILABLE)?;
        budget()?;
        if sftp.realpath(&path).map_err(|_| UNAVAILABLE)? != path {
            return Err(UNAVAILABLE.into());
        }
        if tree.files.len() >= MAX_ENTRIES {
            return Err(UNAVAILABLE.into());
        }
        let mode = stat.perm.ok_or(UNAVAILABLE)?;
        if stat.is_dir() {
            tree.files.insert(
                relative.clone(),
                FileFact {
                    sha256: String::new(),
                    bytes: 0,
                    mode,
                },
            );
            budget()?;
            for child in children(&sftp, &path, &budget)? {
                let name = child
                    .file_name()
                    .and_then(|v| v.to_str())
                    .ok_or(UNAVAILABLE)?;
                if name == "." || name == ".." {
                    continue;
                }
                if child.parent() != Some(path.as_path()) || name.chars().any(char::is_control) {
                    return Err(UNAVAILABLE.into());
                }
                let rel = if relative.is_empty() {
                    name.into()
                } else {
                    format!("{relative}/{name}")
                };
                if pending.len() + tree.files.len() >= MAX_ENTRIES {
                    return Err(UNAVAILABLE.into());
                }
                pending.push((child, rel));
            }
        } else if stat.is_file() {
            let size = stat.size.ok_or(UNAVAILABLE)?;
            if size > MAX_BYTES - tree.bytes {
                return Err(UNAVAILABLE.into());
            }
            budget()?;
            let mut file = sftp.open(&path).map_err(|_| UNAVAILABLE)?;
            let mut count = 0u64;
            let mut hash = Sha256::new();
            loop {
                budget()?;
                let n = file.read(&mut buffer).map_err(|_| UNAVAILABLE)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > size {
                    return Err(UNAVAILABLE.into());
                }
                hash.update(&buffer[..n]);
            }
            budget()?;
            let after = sftp.lstat(&path).map_err(|_| UNAVAILABLE)?;
            if count != size
                || !after.is_file()
                || after.size != stat.size
                || after.mtime != stat.mtime
                || after.perm != stat.perm
            {
                return Err(UNAVAILABLE.into());
            }
            tree.bytes += count;
            tree.files.insert(
                relative,
                FileFact {
                    sha256: hex::encode(hash.finalize()),
                    bytes: count,
                    mode,
                },
            );
        } else {
            return Err(UNAVAILABLE.into());
        }
    }
    budget()?;
    // Directory/filename changes during enumeration must not be silently omitted.
    for (relative, fact) in &tree.files {
        if fact.mode & 0o170000 != 0o040000 {
            continue;
        }
        let path = remote_path(&canonical, relative)?;
        budget()?;
        let children = children(&sftp, &path, &budget)?;
        let actual = children
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .filter(|n| *n != "." && *n != "..")
            .collect::<std::collections::BTreeSet<_>>();
        let prefix = if relative.is_empty() {
            String::new()
        } else {
            format!("{relative}/")
        };
        let expected = tree
            .files
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|p| !p.is_empty() && !p.contains('/'))
            .map(String::from)
            .collect::<std::collections::BTreeSet<_>>();
        if actual
            .into_iter()
            .map(String::from)
            .collect::<std::collections::BTreeSet<_>>()
            != expected
        {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(tree)
}

pub(super) fn artifact_bytes(
    ssh: &ssh2::Session,
    tree: &Tree,
    relative: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    const INVALID: &str =
        "LEARNING_ARTIFACT_UNAVAILABLE:无法保存声明的产物，请检查文件、类型、大小和读取权限";
    let fact = tree.files.get(relative).ok_or(INVALID)?;
    if fact.mode & 0o170000 != 0o100000 || fact.bytes > max_bytes || max_bytes > 32 * 1024 * 1024 {
        return Err(INVALID.into());
    }
    let path = remote_path(&tree.root, relative)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let budget = || -> Result<(), String> {
        ssh.set_timeout(
            execution::remaining_timeout(deadline, Instant::now()).map_err(|_| INVALID)?,
        );
        Ok(())
    };
    budget()?;
    let sftp = ssh.sftp().map_err(|_| INVALID)?;
    budget()?;
    let before = sftp.lstat(&path).map_err(|_| INVALID)?;
    if !before.is_file()
        || before.size != Some(fact.bytes)
        || before.perm != Some(fact.mode)
        || sftp.realpath(&path).map_err(|_| INVALID)? != path
    {
        return Err(INVALID.into());
    }
    budget()?;
    let mut file = sftp.open(&path).map_err(|_| INVALID)?;
    let mut data = Vec::with_capacity(fact.bytes as usize);
    let mut buffer = [0u8; 65536];
    loop {
        budget()?;
        let count = file.read(&mut buffer).map_err(|_| INVALID)?;
        if count == 0 {
            break;
        }
        if data.len() as u64 + count as u64 > fact.bytes {
            return Err(INVALID.into());
        }
        data.extend_from_slice(&buffer[..count]);
    }
    budget()?;
    let after = sftp.lstat(&path).map_err(|_| INVALID)?;
    if data.len() as u64 != fact.bytes
        || sha256(&data) != fact.sha256
        || !after.is_file()
        || after.size != before.size
        || after.perm != before.perm
        || after.mtime != before.mtime
        || sftp.realpath(&path).map_err(|_| INVALID)? != path
    {
        return Err(INVALID.into());
    }
    Ok(data)
}

pub(super) fn saved(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(String, Snapshot), String> {
    let (digest,raw):(String,String)=db.query_row("SELECT digest,detail_json FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=(SELECT MAX(attempt) FROM learning_attempts WHERE session_id=?1 AND step_id=?2) AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1",params![session.session_id,session.current_step],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|LEGACY)?;
    if raw.len() > 32768 || sha256(raw.as_bytes()) != digest {
        return Err(UNAVAILABLE.into());
    }
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|_| UNAVAILABLE)?;
    let snapshot: Snapshot = serde_json::from_value(
        value
            .get("workspaceSnapshot")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or(LEGACY)?,
    )
    .map_err(|_| UNAVAILABLE)?;
    if !snapshot.valid() {
        return Err(UNAVAILABLE.into());
    }
    Ok((digest, snapshot))
}

pub(super) async fn transition(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    session_id: String,
    next_state: String,
    answer: Option<String>,
    reflection: Option<String>,
) -> Result<LocalLearningSession, String> {
    let (
        session,
        attempt,
        lease,
        project_config,
        host,
        subdir,
        expected,
        evidence_digest,
        environment_specs,
        input_paths,
        prerequisite_inputs,
    ) = {
        let mut db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        let session = get_session(&db, &session_id)?;
        let (manifest, lab) = manifest_and_lab(&db, &session)?;
        if theory::is_reflection(&lab)
            || !matches!(
                next_state.as_str(),
                "verifying" | "reflecting" | "completed"
            )
        {
            return transition_step(&mut db, &session_id, &next_state, answer, reflection);
        }
        if !theory::transition(&lab, &session.state, &next_state) {
            return Err("LEARNING_STATE_TRANSITION_INVALID".into());
        }
        require_current_verification(&db, &session)?;
        let prerequisite_inputs = prerequisites::execution_plan(&db, &session, &manifest, &lab)?;
        let (evidence_digest, expected) = saved(&db, &session)?;
        let input_paths = if expected.inputs_sha256.is_some() {
            Some(inputs(&db, &session, &manifest)?)
        } else {
            None
        };
        environment_receipt::require(&expected.environment, &lab.environment_checks)?;
        let work = workbench.db.lock().map_err(|_| UNAVAILABLE)?;
        let (project, host) = project(&work, &session)?;
        if expected.binding_sha256 != binding(&project, &host)? {
            return Err(CHANGED.into());
        }
        let attempt = current_attempt(&db, &session_id, &session.current_step)?;
        let lease = local.actions.acquire(session.project_id)?;
        (
            session,
            attempt,
            lease,
            project,
            host,
            manifest
                .workspace
                .map(|w| w.remote_subdir)
                .unwrap_or_else(|| "tutorial-examples/web-kvm".into()),
            expected,
            evidence_digest,
            lab.environment_checks,
            input_paths,
            prerequisite_inputs,
        )
    };
    let credentials = crate::workbench::credentials::resolve(host.id, SshCredentials::default())?;
    let bound = binding(&project_config, &host)?;
    let root = format!(
        "{}/{}",
        project_config.sdk_dir.trim_end_matches('/'),
        subdir
    );
    let expected_workspace = expected.clone();
    let (lease, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let (ssh, _, _) = crate::workbench::ssh::connect(&host, &credentials)?;
            let tree = observe(&ssh, &root)?;
            if !prerequisite_inputs.is_empty() {
                let paths = input_paths.as_ref().ok_or(UNAVAILABLE)?;
                prerequisites::check_with(
                    &prerequisite_inputs,
                    &source_identity(&tree, paths)?,
                    &bound,
                    |specs| environment_receipt::checked(&ssh, &project_config.sdk_dir, specs),
                )?;
            }
            let environment =
                environment_receipt::checked(&ssh, &project_config.sdk_dir, &environment_specs)?;
            refreeze(
                &expected_workspace,
                &tree,
                &bound,
                environment,
                input_paths.as_deref(),
            )
        })();
        (lease, result)
    })
    .await
    .map_err(|_| UNAVAILABLE)?;
    let _lease = lease;
    compare(&expected, &result?)?;
    let mut db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let work = workbench.db.lock().map_err(|_| UNAVAILABLE)?;
    let (project, host) = project(&work, &session)?;
    if expected.binding_sha256 != binding(&project, &host)? {
        return Err(CHANGED.into());
    }
    ensure_action_context(&db, &session, attempt)?;
    if saved(&db, &session)?.0 != evidence_digest {
        return Err("LEARNING_ACTION_CONTEXT_CHANGED".into());
    }
    transition_step(&mut db, &session_id, &next_state, answer, reflection)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tree() -> Tree {
        Tree {
            root: "/sdk/lesson".into(),
            bytes: 3,
            files: BTreeMap::from([
                (
                    "".into(),
                    FileFact {
                        sha256: String::new(),
                        bytes: 0,
                        mode: 0o40755,
                    },
                ),
                (
                    "main.c".into(),
                    FileFact {
                        sha256: sha256(b"abc"),
                        bytes: 3,
                        mode: 0o100644,
                    },
                ),
            ]),
        }
    }
    #[test]
    fn remote_paths_keep_posix_separators_and_reject_traversal_on_every_host() {
        assert_eq!(
            remote_path("/sdk/test ' 中文", "src/main.c")
                .unwrap()
                .to_str()
                .unwrap(),
            "/sdk/test ' 中文/src/main.c"
        );
        assert_eq!(remote_path("/", "src").unwrap().to_str().unwrap(), "/src");
        for bad in ["../outside", "/outside", "src/../outside"] {
            assert!(remote_path("/sdk", bad).is_err());
        }
    }
    #[test]
    fn snapshot_compares_content_path_permission_root_and_binding() {
        let initial = tree();
        let expected = freeze(&initial, &sha256(b"binding")).unwrap();
        compare(&expected, &freeze(&tree(), &sha256(b"binding")).unwrap()).unwrap();
        for change in 0..6 {
            let mut current = tree();
            match change {
                0 => current.files.get_mut("main.c").unwrap().sha256 = sha256(b"def"),
                1 => {
                    current.files.remove("main.c");
                }
                2 => {
                    current.files.insert(
                        "new.c".into(),
                        FileFact {
                            sha256: sha256(b""),
                            bytes: 0,
                            mode: 0o100644,
                        },
                    );
                }
                3 => current.files.get_mut("main.c").unwrap().mode = 0o100755,
                4 => current.root = "/another/lesson".into(),
                _ => {}
            }
            let observed = freeze(
                &current,
                &sha256(if change == 5 { b"changed" } else { b"binding" }),
            )
            .unwrap();
            assert_eq!(compare(&expected, &observed).unwrap_err(), CHANGED);
        }
    }
    #[test]
    fn input_mutation_during_execution_is_rejected_but_new_outputs_are_allowed() {
        let before = tree();
        let mut after = tree();
        after.files.insert(
            "output.bin".into(),
            FileFact {
                sha256: sha256(b""),
                bytes: 0,
                mode: 0o100644,
            },
        );
        unchanged_inputs(&before, &after, &["main.c".into()]).unwrap();
        after.files.get_mut("main.c").unwrap().sha256 = sha256(b"def");
        assert_eq!(
            unchanged_inputs(&before, &after, &["main.c".into()]).unwrap_err(),
            CHANGED
        );
        assert_eq!(
            unchanged_inputs(&before, &after, &["missing".into()]).unwrap_err(),
            UNAVAILABLE
        );
    }
    #[test]
    fn revisit_source_identity_detects_input_changes_without_invalidating_generated_outputs() {
        let mut observed = tree();
        let paths = vec!["main.c".to_owned()];
        let before = source_identity(&observed, &paths).unwrap();
        observed.files.insert(
            "output.bin".into(),
            FileFact {
                sha256: sha256(b"result"),
                bytes: 6,
                mode: 0o100644,
            },
        );
        assert_eq!(source_identity(&observed, &paths).unwrap(), before);
        observed.files.get_mut("main.c").unwrap().sha256 = sha256(b"changed");
        assert_ne!(source_identity(&observed, &paths).unwrap(), before);
        assert!(source_identity(&observed, &[]).is_err());
        assert_eq!(
            source_identity(&observed, &["missing".into()]).unwrap_err(),
            CHANGED
        );
    }
    #[test]
    fn revisit_deleted_source_remains_a_known_change_before_source_identity() {
        let bound = sha256(b"binding");
        let paths = vec!["main.c".to_owned()];
        let original = tree();
        let expected = freeze_sources(&original, &bound, None, &paths).unwrap();
        assert_eq!(
            refreeze(&expected, &original, &bound, None, Some(&paths)).unwrap(),
            expected
        );
        let mut deleted = tree();
        deleted.files.remove("main.c");
        deleted.bytes = 0;
        assert_eq!(
            refreeze(&expected, &deleted, &bound, None, Some(&paths)).unwrap_err(),
            CHANGED
        );
    }
    #[test]
    fn snapshot_rejects_unknown_version_bad_digest_and_resource_excess() {
        let expected = freeze(&tree(), &sha256(b"binding")).unwrap();
        for case in 0..4 {
            let mut bad = expected.clone();
            match case {
                0 => bad.version = 2,
                1 => bad.tree_sha256 = "invalid".into(),
                2 => bad.entries = MAX_ENTRIES + 1,
                _ => bad.bytes = MAX_BYTES + 1,
            }
            assert_eq!(compare(&expected, &bad).unwrap_err(), UNAVAILABLE);
        }
    }
    fn database() -> (Connection, LocalLearningSession) {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('s','c','1',1,'step','collecting_evidence',1,1)",[]).unwrap();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('s','step',1,'collecting_evidence',1,1)",[]).unwrap();
        let session = get_session(&db, "s").unwrap();
        (db, session)
    }
    #[test]
    fn stored_snapshot_is_hash_checked_current_attempt_only_and_legacy_is_unavailable() {
        let (db, session) = database();
        append_evidence(
            &db,
            &session,
            "command",
            "software",
            "PASS",
            "old",
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(saved(&db, &session).unwrap_err(), LEGACY);
        let expected = freeze(&tree(), &sha256(b"binding")).unwrap();
        append_evidence(
            &db,
            &session,
            "command",
            "software",
            "PASS",
            "observed",
            &serde_json::json!({"workspaceSnapshot":expected}),
        )
        .unwrap();
        assert_eq!(saved(&db, &session).unwrap().1, expected);
        db.execute("UPDATE learning_evidence SET detail_json='{}' WHERE id=(SELECT MAX(id) FROM learning_evidence)",[]).unwrap();
        assert_eq!(saved(&db, &session).unwrap_err(), UNAVAILABLE);
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('s','step',2,'practicing',1,1)",[]).unwrap();
        assert_eq!(saved(&db, &session).unwrap_err(), LEGACY);
    }
    #[test]
    fn recovery_is_available_before_verification_and_after_reflection_but_not_after_completion() {
        for state in ["collecting_evidence", "verifying", "reflecting"] {
            assert!(transition_allowed(state, "practicing"));
        }
        assert!(!transition_allowed("completed", "practicing"));
        assert!(!transition_allowed("reading", "practicing"));
    }
}

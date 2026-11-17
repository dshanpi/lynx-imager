use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use super::types::SshCredentials;
use super::{credentials, ssh, storage, WorkbenchState};

const MANIFEST_NAME: &str = ".lynx-skills.json";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeSkillManifest {
    schema_version: u32,
    skills: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeSkillSyncResult {
    pub provider: String,
    pub root: String,
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub conflicts: Vec<String>,
}

#[tauri::command]
pub async fn workbench_sync_native_skills(
    project_id: i64,
    providers: Vec<String>,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<NativeSkillSyncResult>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sync_native_skills(project_id, providers, &state))
        .await
        .map_err(|_| "SKILLS_SYNC_WORKER_FAILED".to_owned())?
}

fn sync_native_skills(
    project_id: i64,
    providers: Vec<String>,
    state: &WorkbenchState,
) -> Result<Vec<NativeSkillSyncResult>, String> {
    let (host, project) = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let project = storage::project_by_id(&db, project_id).map_err(|error| error.to_string())?;
        let host = storage::host_by_id(&db, project.host_id).map_err(|error| error.to_string())?;
        (host, project)
    };
    if project.sdk_dir.is_empty() || !Path::new(&project.sdk_dir).is_absolute() {
        return Err("SKILLS_SYNC_SDK_PATH_INVALID".to_owned());
    }
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }
    let mut roots = Vec::new();
    for provider in providers {
        let relative = match provider.as_str() {
            "codex" => ".agents/skills",
            "claude" => ".claude/skills",
            "deepseek" => return Err("SKILLS_SYNC_PROVIDER_MCP_ONLY".to_owned()),
            _ => return Err("SKILLS_SYNC_PROVIDER_INVALID".to_owned()),
        };
        if !roots
            .iter()
            .any(|(name, _): &(String, String)| name == &provider)
        {
            roots.push((provider, relative.to_owned()));
        }
    }
    if roots.is_empty() {
        return Err("SKILLS_SYNC_PROVIDER_REQUIRED".to_owned());
    }
    let credentials = credentials::resolve(host.id, SshCredentials::default())?;
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_skills_sync");
    let (session, _, _) = ssh::connect(&host, &credentials)?;
    let sftp = session
        .sftp()
        .map_err(|error| format!("SFTP_OPEN:{error}"))?;
    let documents = crate::skills::native_documents(crate::skills::SkillDomain::Linux)?;
    let mut results = Vec::new();
    for (provider, relative) in roots {
        let root = PathBuf::from(&project.sdk_dir).join(relative);
        validate_remote_root(&root, Path::new(&project.sdk_dir))?;
        ensure_remote_directory(&sftp, &root)?;
        let manifest_path = root.join(MANIFEST_NAME);
        let mut manifest = read_manifest(&sftp, &manifest_path)?;
        let mut result = NativeSkillSyncResult {
            provider,
            root: root.to_string_lossy().into_owned(),
            written: Vec::new(),
            unchanged: Vec::new(),
            conflicts: Vec::new(),
        };
        for document in &documents {
            let directory = root.join(&document.name);
            ensure_remote_directory(&sftp, &directory)?;
            let target = directory.join("SKILL.md");
            let existing = read_remote_file(&sftp, &target)?;
            let owned_hash = manifest.skills.get(&document.name);
            if let Some(content) = existing {
                let existing_hash = digest(&content);
                if existing_hash == document.sha256 {
                    result.unchanged.push(document.name.clone());
                    manifest
                        .skills
                        .insert(document.name.clone(), document.sha256.clone());
                    continue;
                }
                if owned_hash.is_none_or(|hash| hash != &existing_hash) {
                    result.conflicts.push(document.name.clone());
                    continue;
                }
            }
            write_remote_atomic(&sftp, &target, document.content.as_bytes())?;
            manifest
                .skills
                .insert(document.name.clone(), document.sha256.clone());
            result.written.push(document.name.clone());
        }
        manifest.schema_version = 1;
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|_| "SKILLS_SYNC_MANIFEST_INVALID".to_owned())?;
        write_remote_atomic(&sftp, &manifest_path, &manifest_bytes)?;
        results.push(result);
    }
    Ok(results)
}

fn validate_remote_root(root: &Path, sdk_root: &Path) -> Result<(), String> {
    if !root.is_absolute()
        || !root.starts_with(sdk_root)
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("SKILLS_SYNC_PATH_INVALID".to_owned());
    }
    Ok(())
}

fn ensure_remote_directory(sftp: &ssh2::Sftp, path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::RootDir) || sftp.stat(&current).is_ok() {
            continue;
        }
        sftp.mkdir(&current, 0o700)
            .map_err(|error| format!("SKILLS_SYNC_MKDIR:{error}"))?;
    }
    Ok(())
}

fn read_manifest(sftp: &ssh2::Sftp, path: &Path) -> Result<NativeSkillManifest, String> {
    match read_remote_file(sftp, path)? {
        None => Ok(NativeSkillManifest::default()),
        Some(content) => {
            serde_json::from_slice(&content).map_err(|_| "SKILLS_SYNC_MANIFEST_INVALID".to_owned())
        }
    }
}

fn read_remote_file(sftp: &ssh2::Sftp, path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match sftp.open(path) {
        Ok(file) => file,
        Err(error) if error.code() == ssh2::ErrorCode::SFTP(2) => return Ok(None),
        Err(error) => return Err(format!("SKILLS_SYNC_READ:{error}")),
    };
    let mut content = Vec::new();
    file.take(MAX_MANIFEST_BYTES as u64)
        .read_to_end(&mut content)
        .map_err(|error| format!("SKILLS_SYNC_READ:{error}"))?;
    Ok(Some(content))
}

fn write_remote_atomic(sftp: &ssh2::Sftp, target: &Path, content: &[u8]) -> Result<(), String> {
    let temporary = target.with_extension("md.lynx-new");
    let mut file = sftp
        .create(&temporary)
        .map_err(|error| format!("SKILLS_SYNC_WRITE:{error}"))?;
    file.write_all(content)
        .and_then(|_| file.flush())
        .map_err(|error| format!("SKILLS_SYNC_WRITE:{error}"))?;
    drop(file);
    sftp.rename(
        &temporary,
        target,
        Some(ssh2::RenameFlags::OVERWRITE | ssh2::RenameFlags::ATOMIC),
    )
    .map_err(|error| format!("SKILLS_SYNC_RENAME:{error}"))
}

fn digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_roots_must_remain_below_the_saved_sdk() {
        assert!(
            validate_remote_root(Path::new("/srv/sdk/.agents/skills"), Path::new("/srv/sdk"))
                .is_ok()
        );
        assert!(validate_remote_root(Path::new("/srv/other"), Path::new("/srv/sdk")).is_err());
        assert!(validate_remote_root(Path::new("relative"), Path::new("/srv/sdk")).is_err());
    }
}

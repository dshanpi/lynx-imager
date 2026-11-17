use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::auth::state::AuthState;

use super::{
    commands::{fetch_learning_workspace_credential, LearningWorkspaceCredential},
    package::{safe_relative_path, verify_package},
};

const MAX_WORKSPACE_BYTES: usize = 64 * 1024 * 1024;
const MAX_WORKSPACE_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WORKSPACE_ENTRIES: usize = 8192;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLearningWorkspace {
    pub enrollment_id: String,
    pub unit_key: String,
    pub commit: String,
    pub sha256: String,
    pub install_path: String,
    pub installed_at: i64,
    pub reused: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceMarker {
    enrollment_id: String,
    unit_key: String,
    commit: String,
    sha256: String,
    installed_at: i64,
}

fn extract_workspace_zip(data: &[u8], destination: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|_| "LEARNING_WORKSPACE_INVALID_ARCHIVE:代码快照不是有效 ZIP".to_owned())?;
    if archive.is_empty() || archive.len() > MAX_WORKSPACE_ENTRIES {
        return Err("LEARNING_WORKSPACE_INVALID_ARCHIVE:代码快照条目数量无效".to_owned());
    }
    let mut total_unpacked = 0_u64;
    let mut names = HashSet::<PathBuf>::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "LEARNING_WORKSPACE_INVALID_ARCHIVE:无法读取代码快照".to_owned())?;
        let relative = safe_relative_path(entry.name())
            .map_err(|_| "LEARNING_WORKSPACE_UNSAFE_PATH:代码快照包含不安全路径".to_owned())?;
        if !names.insert(relative) {
            return Err("LEARNING_WORKSPACE_INVALID_ARCHIVE:代码快照包含重复路径".to_owned());
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("LEARNING_WORKSPACE_UNSAFE_PATH:代码快照禁止包含符号链接".to_owned());
        }
        total_unpacked = total_unpacked
            .checked_add(entry.size())
            .ok_or_else(|| "LEARNING_WORKSPACE_TOO_LARGE:代码快照解压体积溢出".to_owned())?;
        if total_unpacked > MAX_WORKSPACE_UNPACKED_BYTES {
            return Err("LEARNING_WORKSPACE_TOO_LARGE:代码快照解压体积超过 512MB".to_owned());
        }
    }
    fs::create_dir_all(destination)
        .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法创建代码临时目录".to_owned())?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "LEARNING_WORKSPACE_INVALID_ARCHIVE:无法读取代码快照".to_owned())?;
        let relative = safe_relative_path(entry.name())
            .map_err(|_| "LEARNING_WORKSPACE_UNSAFE_PATH:代码快照包含不安全路径".to_owned())?;
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法创建代码目录".to_owned())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法创建代码目录".to_owned())?;
        }
        let mut file = File::create(&output)
            .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法写入代码文件".to_owned())?;
        io::copy(&mut entry, &mut file)
            .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:解压代码快照失败".to_owned())?;
        file.flush()
            .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:保存代码文件失败".to_owned())?;
    }
    Ok(())
}

fn install_workspace(
    root: &Path,
    enrollment_id: &str,
    unit_key: &str,
    credential: &LearningWorkspaceCredential,
    data: &[u8],
) -> Result<InstalledLearningWorkspace, String> {
    let digest = verify_package(data, &credential.sha256, &credential.signature)
        .map_err(|error| error.replace("LAB_PACKAGE_", "LEARNING_WORKSPACE_"))?;
    let workspaces = root.join("learning").join("workspaces");
    let target = workspaces
        .join(enrollment_id)
        .join(unit_key)
        .join(&credential.commit);
    if target.exists() {
        let marker = fs::read(target.join(".lynx-workspace.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<WorkspaceMarker>(&bytes).ok());
        if let Some(marker) = marker.filter(|item| {
            item.enrollment_id == enrollment_id
                && item.unit_key == unit_key
                && item.commit == credential.commit
                && item.sha256 == digest
        }) {
            return Ok(InstalledLearningWorkspace {
                enrollment_id: marker.enrollment_id,
                unit_key: marker.unit_key,
                commit: marker.commit,
                sha256: marker.sha256,
                install_path: target.to_string_lossy().into_owned(),
                installed_at: marker.installed_at,
                reused: true,
            });
        }
        return Err(
            "LEARNING_WORKSPACE_INSTALL_CONFLICT:本地代码目录已存在且完整性记录不匹配".to_owned(),
        );
    }
    let staging_root = workspaces.join(".staging");
    fs::create_dir_all(&staging_root)
        .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法创建代码目录".to_owned())?;
    let staging = staging_root.join(format!("{}-{}", std::process::id(), rand::random::<u64>()));
    if let Err(error) = extract_workspace_zip(data, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let installed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let marker = WorkspaceMarker {
        enrollment_id: enrollment_id.to_owned(),
        unit_key: unit_key.to_owned(),
        commit: credential.commit.clone(),
        sha256: digest.clone(),
        installed_at,
    };
    let marker_json = serde_json::to_vec(&marker)
        .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法生成完整性记录".to_owned())?;
    fs::write(staging.join(".lynx-workspace.json"), marker_json)
        .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法保存完整性记录".to_owned())?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法创建代码目录".to_owned())?;
    }
    fs::rename(&staging, &target).map_err(|_| {
        let _ = fs::remove_dir_all(&staging);
        "LEARNING_WORKSPACE_INSTALL_FAILED:代码快照原子安装失败".to_owned()
    })?;
    Ok(InstalledLearningWorkspace {
        enrollment_id: enrollment_id.to_owned(),
        unit_key: unit_key.to_owned(),
        commit: credential.commit.clone(),
        sha256: digest,
        install_path: target.to_string_lossy().into_owned(),
        installed_at,
        reused: false,
    })
}

async fn download_workspace(
    http: &reqwest::Client,
    credential: &LearningWorkspaceCredential,
) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(&credential.url)
        .map_err(|_| "LEARNING_WORKSPACE_CREDENTIAL_INVALID:代码快照下载地址无效".to_owned())?;
    let secure = url.scheme() == "https";
    let local_debug = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if !secure && !local_debug {
        return Err("LEARNING_WORKSPACE_CREDENTIAL_INVALID:代码快照必须通过 HTTPS 下载".to_owned());
    }
    if credential.expires_at <= 0 {
        return Err("LEARNING_WORKSPACE_CREDENTIAL_INVALID:代码快照下载凭据已失效".to_owned());
    }
    let response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "LEARNING_WORKSPACE_DOWNLOAD_FAILED:代码快照下载失败".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "LEARNING_WORKSPACE_DOWNLOAD_FAILED:代码快照下载返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length == 0 || length > MAX_WORKSPACE_BYTES as u64)
    {
        return Err("LEARNING_WORKSPACE_TOO_LARGE:代码快照下载大小超过 64MB".to_owned());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| "LEARNING_WORKSPACE_DOWNLOAD_FAILED:代码快照下载中断".to_owned())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_WORKSPACE_BYTES {
            return Err("LEARNING_WORKSPACE_TOO_LARGE:代码快照下载大小超过 64MB".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("LEARNING_WORKSPACE_DOWNLOAD_FAILED:代码快照为空".to_owned());
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn learning_install_unit_workspace(
    app: AppHandle,
    state: State<'_, std::sync::Arc<AuthState>>,
    enrollment_id: String,
    unit_key: String,
) -> Result<InstalledLearningWorkspace, String> {
    let credential = fetch_learning_workspace_credential(&state, &enrollment_id, &unit_key).await?;
    let data = download_workspace(&state.http, &credential).await?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:无法定位 LYNX 数据目录".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        install_workspace(&root, &enrollment_id, &unit_key, &credential, &data)
    })
    .await
    .map_err(|_| "LEARNING_WORKSPACE_INSTALL_FAILED:代码安装任务异常退出".to_owned())?
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::extract_workspace_zip;

    #[test]
    fn workspace_zip_rejects_escape_and_symlink() {
        let mut archive = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut archive);
            let mut writer = zip::ZipWriter::new(cursor);
            writer
                .start_file("../outside.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"bad").unwrap();
            writer.finish().unwrap();
        }
        let target =
            std::env::temp_dir().join(format!("lynx-workspace-test-{}", rand::random::<u64>()));
        assert!(extract_workspace_zip(&archive, &target).is_err());
        assert!(!target.join("outside.txt").exists());
    }
}

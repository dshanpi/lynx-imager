use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};

use crate::auth::state::AuthState;

use super::commands::{fetch_lab_download_credential, LabDownloadCredential};

const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLabPackage {
    pub lab_key: String,
    pub version: String,
    pub sha256: String,
    pub installed_at: i64,
}

#[derive(Debug, Deserialize)]
struct PackageManifestIdentity {
    lab_id: String,
    version: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct InstalledPackageMarker {
    lab_key: String,
    version: String,
    sha256: String,
    installed_at: i64,
}

fn package_public_key() -> Result<VerifyingKey, String> {
    let encoded = option_env!("LYNX_PACKAGE_SIGNING_PUBLIC_KEY")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "LAB_PACKAGE_PUBLIC_KEY_MISSING:当前 LYNX 构建未配置实验包发布公钥，禁止安装".to_owned()
        })?;
    parse_public_key(encoded)
}

fn parse_public_key(encoded: &str) -> Result<VerifyingKey, String> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "LAB_PACKAGE_PUBLIC_KEY_INVALID:实验包发布公钥不是有效 Base64".to_owned())?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "LAB_PACKAGE_PUBLIC_KEY_INVALID:实验包发布公钥长度无效".to_owned())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| "LAB_PACKAGE_PUBLIC_KEY_INVALID:实验包发布公钥无效".to_owned())
}

pub(crate) fn verify_package(
    data: &[u8],
    expected_sha256: &str,
    signature: &str,
) -> Result<String, String> {
    verify_package_with_key(data, expected_sha256, signature, &package_public_key()?)
}

fn verify_package_with_key(
    data: &[u8],
    expected_sha256: &str,
    signature: &str,
    public_key: &VerifyingKey,
) -> Result<String, String> {
    let digest = Sha256::digest(data);
    let actual = hex::encode(digest);
    if !actual.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err("LAB_PACKAGE_HASH_MISMATCH:实验包 SHA-256 校验失败".to_owned());
    }
    let signed = STANDARD
        .decode(signature.trim())
        .map_err(|_| "LAB_PACKAGE_SIGNATURE_INVALID:实验包签名格式无效".to_owned())?;
    let signature = Signature::from_slice(&signed)
        .map_err(|_| "LAB_PACKAGE_SIGNATURE_INVALID:实验包签名长度无效".to_owned())?;
    public_key
        .verify(&digest, &signature)
        .map_err(|_| "LAB_PACKAGE_SIGNATURE_INVALID:实验包签名验证失败".to_owned())?;
    Ok(actual)
}

pub(crate) fn safe_relative_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('\0') || name.contains('\\') {
        return Err("LAB_PACKAGE_UNSAFE_PATH:实验包包含不安全路径".to_owned());
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("LAB_PACKAGE_UNSAFE_PATH:实验包包含路径穿越".to_owned());
    }
    Ok(path.to_path_buf())
}

fn extract_verified_zip(
    data: &[u8],
    destination: &Path,
    expected_lab: &str,
    expected_version: &str,
) -> Result<(), String> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|_| "LAB_PACKAGE_INVALID_ARCHIVE:实验包不是有效 ZIP".to_owned())?;
    if archive.is_empty() || archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("LAB_PACKAGE_INVALID_ARCHIVE:实验包条目数量无效".to_owned());
    }
    let mut total_unpacked = 0_u64;
    let mut manifest_name = None;
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "LAB_PACKAGE_INVALID_ARCHIVE:无法读取实验包条目".to_owned())?;
        let relative = safe_relative_path(entry.name())?;
        if !names.insert(relative.clone()) {
            return Err("LAB_PACKAGE_INVALID_ARCHIVE:实验包包含重复路径".to_owned());
        }
        if relative.components().count() == 1
            && matches!(
                relative.file_name().and_then(|value| value.to_str()),
                Some("manifest.yaml" | "manifest.yml" | "manifest.json")
            )
        {
            manifest_name = Some(relative.clone());
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("LAB_PACKAGE_UNSAFE_PATH:实验包禁止包含符号链接".to_owned());
        }
        total_unpacked = total_unpacked
            .checked_add(entry.size())
            .ok_or_else(|| "LAB_PACKAGE_TOO_LARGE:实验包解压体积溢出".to_owned())?;
        if total_unpacked > MAX_UNPACKED_BYTES {
            return Err("LAB_PACKAGE_TOO_LARGE:实验包解压体积超过 512MB".to_owned());
        }
    }
    let manifest_name = manifest_name
        .ok_or_else(|| "LAB_PACKAGE_MANIFEST_MISSING:实验包根目录缺少 Manifest".to_owned())?;
    let manifest_path = manifest_name
        .to_str()
        .ok_or_else(|| "LAB_PACKAGE_MANIFEST_INVALID:Manifest 文件名无效".to_owned())?;
    let mut manifest_entry = archive
        .by_name(manifest_path)
        .map_err(|_| "LAB_PACKAGE_MANIFEST_INVALID:无法读取 Manifest".to_owned())?;
    if manifest_entry.size() > 1024 * 1024 {
        return Err("LAB_PACKAGE_MANIFEST_INVALID:Manifest 超过 1MB".to_owned());
    }
    let mut manifest = Vec::with_capacity(manifest_entry.size() as usize);
    io::copy(&mut manifest_entry, &mut manifest)
        .map_err(|_| "LAB_PACKAGE_MANIFEST_INVALID:读取 Manifest 失败".to_owned())?;
    drop(manifest_entry);
    let identity: PackageManifestIdentity =
        if manifest_name.extension().and_then(|value| value.to_str()) == Some("json") {
            serde_json::from_slice(&manifest)
                .map_err(|_| "LAB_PACKAGE_MANIFEST_INVALID:Manifest JSON 格式无效".to_owned())?
        } else {
            serde_yaml::from_slice(&manifest)
                .map_err(|_| "LAB_PACKAGE_MANIFEST_INVALID:Manifest YAML 格式无效".to_owned())?
        };
    if identity.lab_id != expected_lab || identity.version != expected_version {
        return Err(
            "LAB_PACKAGE_MANIFEST_MISMATCH:Manifest 的实验或版本与下载凭据不一致".to_owned(),
        );
    }

    fs::create_dir_all(destination)
        .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法创建实验包临时目录".to_owned())?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "LAB_PACKAGE_INVALID_ARCHIVE:无法读取实验包条目".to_owned())?;
        let relative = safe_relative_path(entry.name())?;
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法创建实验包目录".to_owned())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法创建实验包目录".to_owned())?;
        }
        let mut file = File::create(&output)
            .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法写入实验包文件".to_owned())?;
        io::copy(&mut entry, &mut file)
            .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:解压实验包失败".to_owned())?;
        file.flush()
            .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:保存实验包失败".to_owned())?;
    }
    Ok(())
}

fn install_verified_package(
    root: &Path,
    lab_key: &str,
    credential: &LabDownloadCredential,
    data: &[u8],
) -> Result<InstalledLabPackage, String> {
    let digest = verify_package(data, &credential.sha256, &credential.signature)?;
    let packages = root.join("learning").join("packages");
    let staging_root = packages.join(".staging");
    fs::create_dir_all(&staging_root)
        .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法创建实验包目录".to_owned())?;
    let nonce = format!("{}-{}", std::process::id(), rand::random::<u64>());
    let staging = staging_root.join(nonce);
    let target = packages.join(lab_key).join(&credential.version);
    if target.exists() {
        let marker = fs::read(target.join(".lynx-package.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstalledPackageMarker>(&bytes).ok());
        if let Some(marker) = marker.filter(|item| item.sha256 == digest) {
            return Ok(InstalledLabPackage {
                lab_key: marker.lab_key,
                version: marker.version,
                sha256: marker.sha256,
                installed_at: marker.installed_at,
            });
        }
        return Err(
            "LAB_PACKAGE_INSTALL_CONFLICT:同版本本地目录已存在但完整性记录不匹配".to_owned(),
        );
    }
    if let Err(error) = extract_verified_zip(data, &staging, lab_key, &credential.version) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法创建实验安装目录".to_owned())?;
    }
    let installed_at = chrono_timestamp();
    let marker = InstalledPackageMarker {
        lab_key: lab_key.to_owned(),
        version: credential.version.clone(),
        sha256: digest.clone(),
        installed_at,
    };
    let marker_json = serde_json::to_vec(&marker)
        .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法生成完整性记录".to_owned())?;
    fs::write(staging.join(".lynx-package.json"), marker_json)
        .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法保存完整性记录".to_owned())?;
    fs::rename(&staging, &target).map_err(|_| {
        let _ = fs::remove_dir_all(&staging);
        "LAB_PACKAGE_INSTALL_FAILED:实验包原子安装失败".to_owned()
    })?;
    Ok(InstalledLabPackage {
        lab_key: lab_key.to_owned(),
        version: credential.version.clone(),
        sha256: digest,
        installed_at,
    })
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

async fn download_package(
    http: &reqwest::Client,
    credential: &LabDownloadCredential,
) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(&credential.url)
        .map_err(|_| "LAB_PACKAGE_CREDENTIAL_INVALID:实验包下载地址无效".to_owned())?;
    let secure = url.scheme() == "https";
    let local_debug = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if !secure && !local_debug {
        return Err("LAB_PACKAGE_CREDENTIAL_INVALID:实验包必须通过 HTTPS 下载".to_owned());
    }
    let response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "LAB_PACKAGE_DOWNLOAD_FAILED:实验包下载失败".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "LAB_PACKAGE_DOWNLOAD_FAILED:实验包下载返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length == 0 || length > MAX_PACKAGE_BYTES as u64)
    {
        return Err("LAB_PACKAGE_TOO_LARGE:实验包下载大小超过 64MB".to_owned());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "LAB_PACKAGE_DOWNLOAD_FAILED:实验包下载中断".to_owned())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_PACKAGE_BYTES {
            return Err("LAB_PACKAGE_TOO_LARGE:实验包下载大小超过 64MB".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("LAB_PACKAGE_TOO_LARGE:实验包下载大小必须在 1B 到 64MB 之间".to_owned());
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn learning_install_lab_package(
    app: AppHandle,
    state: State<'_, std::sync::Arc<AuthState>>,
    identifier: String,
) -> Result<InstalledLabPackage, String> {
    let credential = fetch_lab_download_credential(&state, &identifier).await?;
    let data = download_package(&state.http, &credential).await?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:无法定位 LYNX 数据目录".to_owned())?;
    let lab_key = identifier.trim().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        install_verified_package(&root, &lab_key, &credential, &data)
    })
    .await
    .map_err(|_| "LAB_PACKAGE_INSTALL_FAILED:实验包安装任务异常退出".to_owned())?
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    use super::{extract_verified_zip, safe_relative_path, verify_package_with_key};

    #[test]
    fn package_paths_reject_escape_and_windows_separators() {
        for value in ["../secret", "/etc/passwd", "folder\\secret", "a/../../b"] {
            assert!(safe_relative_path(value).is_err(), "accepted {value}");
        }
        assert!(safe_relative_path("workspace/src/hello.c").is_ok());
    }

    #[test]
    fn archive_requires_root_manifest() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            writer
                .start_file(
                    "workspace/hello.c",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap();
        }
        let root = std::env::temp_dir().join(format!(
            "lynx-package-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let error =
            extract_verified_zip(cursor.get_ref(), &root, "char-device-01", "1.0.0").unwrap_err();
        assert!(error.starts_with("LAB_PACKAGE_MANIFEST_MISSING:"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn signature_matches_backend_digest_contract() {
        let data = b"signed zip bytes";
        let digest = Sha256::digest(data);
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = STANDARD.encode(key.sign(&digest).to_bytes());
        let verified =
            verify_package_with_key(data, &hex::encode(digest), &signature, &key.verifying_key())
                .expect("backend-compatible signature should verify");
        assert_eq!(verified, hex::encode(digest));
        assert!(verify_package_with_key(
            b"changed",
            &hex::encode(digest),
            &signature,
            &key.verifying_key()
        )
        .is_err());
    }

    #[test]
    fn archive_manifest_must_match_download_target() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            writer
                .start_file("manifest.yaml", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer
                .write_all(b"lab_id: another-lab\nversion: 1.0.0\n")
                .unwrap();
            writer.finish().unwrap();
        }
        let root = std::env::temp_dir().join(format!(
            "lynx-package-mismatch-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let error =
            extract_verified_zip(cursor.get_ref(), &root, "char-device-01", "1.0.0").unwrap_err();
        assert!(error.starts_with("LAB_PACKAGE_MANIFEST_MISMATCH:"));
        let _ = std::fs::remove_dir_all(root);
    }
}

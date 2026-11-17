//! Public firmware metadata and verified cache, separate from the app updater.
use super::{
    firmware_applications as package, firmware_lifecycle as lifecycle,
    firmware_release_source as source, WorkbenchState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwareVersion {
    pub manifest: package::FirmwareCandidateManifest,
    pub image_asset_id: u64,
    pub status: String,
    pub withdrawal_reason: String,
    pub notes: String,
    pub documentation_url: String,
    pub distribution_license: String,
    pub hardware: Value,
    pub recipe: Value,
    pub validation: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwareIndex {
    pub schema_version: u32,
    pub repository: String,
    pub release_tag: String,
    pub channel: String,
    pub entries: Vec<FirmwareVersion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogVersion {
    pub release_id: u64,
    pub tag: String,
    pub index_asset_id: u64,
    pub index_sha256: String,
    pub image_url: String,
    pub channel: String,
    pub entry: FirmwareVersion,
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("LYNX-Firmware/1")
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() > 4 {
                return a.error("FIRMWARE_REDIRECT_LIMIT");
            }
            let u = a.url();
            if u.scheme() == "https"
                && matches!(
                    u.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
                && u.username().is_empty()
                && u.password().is_none()
            {
                a.follow()
            } else {
                a.error("FIRMWARE_REDIRECT_REJECTED")
            }
        }))
        .build()
        .map_err(|_| "FIRMWARE_NETWORK_FAILED".into())
}
fn validate_public_url(value: &str) -> Result<(), String> {
    let u = reqwest::Url::parse(value).map_err(|_| "FIRMWARE_URL_INVALID")?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some_and(|p| p != 443)
        || !matches!(
            u.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
    {
        return Err("FIRMWARE_URL_INVALID".into());
    }
    Ok(())
}
pub async fn response_bytes(mut r: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if r.content_length().is_some_and(|n| n > limit as u64) {
        return Err("FIRMWARE_RESPONSE_TOO_LARGE".into());
    }
    let mut b = Vec::new();
    while let Some(c) = r
        .chunk()
        .await
        .map_err(|_| "FIRMWARE_DOWNLOAD_INTERRUPTED")?
    {
        if b.len() + c.len() > limit {
            return Err("FIRMWARE_RESPONSE_TOO_LARGE".into());
        }
        b.extend_from_slice(&c);
    }
    Ok(b)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Withdrawal {
    schema_version: u32,
    withdrawn: bool,
    reason: String,
    at: i64,
}
fn withdrawal_reason(bytes: &[u8]) -> Result<String, String> {
    let w: Withdrawal = serde_json::from_slice(bytes).map_err(|_| "FIRMWARE_WITHDRAWAL_INVALID")?;
    if w.schema_version != 1
        || !w.withdrawn
        || w.reason.trim().is_empty()
        || w.reason.len() > 2000
        || w.at <= 0
    {
        return Err("FIRMWARE_WITHDRAWAL_INVALID".into());
    }
    Ok(w.reason)
}
pub async fn bounded(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_public_url(url)?;
    let mut r = client()?
        .get(url)
        .send()
        .await
        .map_err(|_| "FIRMWARE_NETWORK_FAILED")?;
    if !r.status().is_success() {
        return Err(format!("FIRMWARE_HTTP_{}", r.status().as_u16()));
    }
    if r.content_length().is_some_and(|n| n > limit as u64) {
        return Err("FIRMWARE_RESPONSE_TOO_LARGE".into());
    }
    let mut bytes = vec![];
    while let Some(chunk) = r
        .chunk()
        .await
        .map_err(|_| "FIRMWARE_DOWNLOAD_INTERRUPTED")?
    {
        if bytes.len() + chunk.len() > limit {
            return Err("FIRMWARE_RESPONSE_TOO_LARGE".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub fn validate_index(index: &FirmwareIndex, tag: &str, prerelease: bool) -> Result<(), String> {
    if index.schema_version != 1
        || index.repository != source::REPOSITORY
        || index.release_tag != tag
        || index.channel != if prerelease { "test" } else { "stable" }
        || index.entries.is_empty()
        || index.entries.len() > 30
    {
        return Err("FIRMWARE_INDEX_INVALID".into());
    }
    let mut ids = std::collections::HashSet::new();
    for e in &index.entries {
        package::validate_manifest(&e.manifest)?;
        if !ids.insert((&e.manifest.application_id, &e.manifest.version))
            || !matches!(e.status.as_str(), "active" | "withdrawn")
            || e.image_asset_id == 0
            || e.notes.is_empty()
            || e.notes.len() > 10000
            || e.distribution_license.trim().is_empty()
            || e.distribution_license.len() > 200
            || e.withdrawal_reason.len() > 2000
            || (e.status == "withdrawn" && e.withdrawal_reason.trim().is_empty())
        {
            return Err("FIRMWARE_INDEX_INVALID".into());
        }
        if !e
            .documentation_url
            .starts_with("https://zephyr.100ask.net/docs/yuzukineko/")
            || e.hardware["model"] != "yuzukineko"
            || e.hardware["chip"] != "f101"
            || e.hardware["capacityBytes"] != 16777216
            || e.hardware["display"] != "1024x768"
            || e.hardware["touch"] != "gt967"
            || e.recipe["id"] != "nekobox-xip-v1"
        {
            return Err("FIRMWARE_INDEX_COMPATIBILITY_INVALID".into());
        }
    }
    Ok(())
}
pub async fn catalog(channel: &str) -> Result<Vec<CatalogVersion>, String> {
    if !matches!(channel, "stable" | "test") {
        return Err("FIRMWARE_CHANNEL_INVALID".into());
    }
    let releases = source::fetch_releases().await?;
    let mut result = vec![];
    for release in releases {
        if release.draft || release.prerelease != (channel == "test") {
            continue;
        }
        let indices: Vec<_> = release
            .assets
            .iter()
            .filter(|a| a.name == source::INDEX_NAME)
            .collect();
        if indices.is_empty() {
            continue;
        }
        if indices.len() != 1 {
            return Err("FIRMWARE_INDEX_INVALID".into());
        }
        let asset = indices[0];
        let expected = source::release_url(&release.tag_name, Some(source::INDEX_NAME))?;
        if asset.state != "uploaded"
            || asset.size == 0
            || asset.size > 256 * 1024
            || asset.browser_download_url != expected
        {
            return Err("FIRMWARE_INDEX_INVALID".into());
        }
        let bytes = bounded(&expected, 256 * 1024).await?;
        if bytes.len() as u64 != asset.size {
            return Err("FIRMWARE_INDEX_CHANGED".into());
        }
        let index: FirmwareIndex =
            serde_json::from_slice(&bytes).map_err(|_| "FIRMWARE_INDEX_INVALID")?;
        validate_index(&index, &release.tag_name, release.prerelease)?;
        let withdrawal = release
            .assets
            .iter()
            .filter(|a| a.name == "lynx-withdrawal.json")
            .collect::<Vec<_>>();
        if withdrawal.len() > 1 {
            return Err("FIRMWARE_WITHDRAWAL_INVALID".into());
        }
        let reason = if let Some(a) = withdrawal.first() {
            let u = source::release_url(&release.tag_name, Some("lynx-withdrawal.json"))?;
            if a.state != "uploaded" || a.size == 0 || a.size > 8192 || a.browser_download_url != u
            {
                return Err("FIRMWARE_WITHDRAWAL_INVALID".into());
            }
            let bytes = bounded(&u, 8192).await?;
            if bytes.len() as u64 != a.size {
                return Err("FIRMWARE_WITHDRAWAL_INVALID".into());
            }
            Some(withdrawal_reason(&bytes)?)
        } else {
            None
        };
        for mut entry in index.entries {
            if let Some(reason) = &reason {
                entry.status = "withdrawn".into();
                entry.withdrawal_reason = reason.clone();
            }
            let image = release
                .assets
                .iter()
                .find(|a| a.id == entry.image_asset_id)
                .ok_or("FIRMWARE_IMAGE_ASSET_MISSING")?;
            let image_url =
                source::release_url(&release.tag_name, Some(&entry.manifest.image.file_name))?;
            if image.name != entry.manifest.image.file_name
                || image.size != entry.manifest.image.size_bytes
                || image.state != "uploaded"
                || image.browser_download_url != image_url
            {
                return Err("FIRMWARE_IMAGE_ASSET_MISMATCH".into());
            }
            result.push(CatalogVersion {
                release_id: release.id,
                tag: release.tag_name.clone(),
                index_asset_id: asset.id,
                index_sha256: lifecycle::digest(&bytes),
                image_url,
                channel: channel.into(),
                entry,
            });
        }
    }
    Ok(result)
}
#[tauri::command]
pub async fn workbench_firmware_catalog(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    channel: String,
) -> Result<Value, String> {
    let d = lifecycle::board(&state, device_id)?;
    let entries = tokio::time::timeout(Duration::from_secs(60), catalog(&channel))
        .await
        .map_err(|_| "FIRMWARE_RELEASE_TIMEOUT")??;
    if lifecycle::board(&state, device_id)?.board_uid != d.board_uid {
        return Err("FIRMWARE_RELEASE_SCOPE_CHANGED".into());
    }
    Ok(
        json!({"deviceId":device_id,"boardUid":d.board_uid,"checkedAt":lifecycle::now(),"expiresAt":lifecycle::now()+600,"channel":channel,"entries":entries}),
    )
}
pub async fn ensure_current_origin(c: &lifecycle::Candidate) -> Result<(), String> {
    let Some(origin) = &c.origin else {
        return Ok(());
    };
    let old: CatalogVersion =
        serde_json::from_value(origin.clone()).map_err(|_| "FIRMWARE_ORIGIN_INVALID")?;
    let entries = tokio::time::timeout(Duration::from_secs(60), catalog(&old.channel))
        .await
        .map_err(|_| "FIRMWARE_RELEASE_TIMEOUT")??;
    let current = entries
        .iter()
        .find(|v| {
            v.release_id == old.release_id
                && v.entry.manifest.application_id == old.entry.manifest.application_id
                && v.entry.manifest.version == old.entry.manifest.version
        })
        .ok_or("FIRMWARE_VERSION_WITHDRAWN_OR_MISSING")?;
    if current.entry.status != "active" {
        return Err("FIRMWARE_VERSION_WITHDRAWN".into());
    }
    if lifecycle::encode(&current)? != lifecycle::encode(&old)? {
        return Err("FIRMWARE_VERSION_CHANGED_REFRESH_REQUIRED".into());
    }
    Ok(())
}

async fn download_image(
    app: &AppHandle,
    version: &CatalogVersion,
) -> Result<std::path::PathBuf, String> {
    validate_public_url(&version.image_url)?;
    let root = lifecycle::cache_root(app)?.join("downloads");
    std::fs::create_dir_all(&root).map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?;
    let expected = &version.entry.manifest.image;
    let dir = root.join(expected.sha256.to_lowercase());
    std::fs::create_dir_all(&dir).map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?;
    if std::fs::symlink_metadata(&root)
        .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?
        .file_type()
        .is_symlink()
        || std::fs::symlink_metadata(&dir)
            .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?
            .file_type()
            .is_symlink()
    {
        return Err("FIRMWARE_CACHE_UNAVAILABLE".into());
    }
    let target = dir.join(&expected.file_name);
    if target.exists()
        && package::verify_file(
            &dir.canonicalize()
                .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?,
            expected,
        )
        .is_ok()
    {
        return Ok(target);
    }
    let partial = dir.join(format!("{}.part", lifecycle::id("download")));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .await
        .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
    let mut response = client()?
        .get(&version.image_url)
        .send()
        .await
        .map_err(|_| "FIRMWARE_NETWORK_FAILED")?;
    if !response.status().is_success() {
        return Err("FIRMWARE_DOWNLOAD_FAILED".into());
    }
    let mut hash = Sha256::new();
    let mut size = 0u64;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|_| "FIRMWARE_DOWNLOAD_INTERRUPTED")?
    {
        size += bytes.len() as u64;
        if size > expected.size_bytes {
            return Err("FIRMWARE_DOWNLOAD_SIZE_MISMATCH".into());
        }
        hash.update(&bytes);
        file.write_all(&bytes)
            .await
            .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
    }
    file.sync_all()
        .await
        .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
    drop(file);
    if size != expected.size_bytes || hex::encode(hash.finalize()) != expected.sha256.to_lowercase()
    {
        return Err("FIRMWARE_DOWNLOAD_HASH_MISMATCH".into());
    }
    if target.exists() {
        std::fs::remove_file(&target).map_err(|_| "FIRMWARE_CACHE_REPLACE_FAILED")?;
    }
    std::fs::rename(&partial, &target).map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
    package::verify_file(
        &dir.canonicalize()
            .map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?,
        expected,
    )?;
    Ok(target)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadRequest {
    release_id: u64,
    application_id: String,
    version: String,
    channel: String,
    loader_path: String,
}
#[tauri::command]
pub async fn workbench_firmware_download(
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    request: DownloadRequest,
) -> Result<lifecycle::Candidate, String> {
    let DownloadRequest {
        release_id,
        application_id,
        version,
        channel,
        loader_path,
    } = request;
    let d = lifecycle::board(&state, device_id)?;
    let task_id = lifecycle::id("fwdownload");
    super::commands::insert_task(&state, &task_id, "transfer", None)?;
    let result = async {
        let current = tokio::time::timeout(Duration::from_secs(60), catalog(&channel))
            .await
            .map_err(|_| "FIRMWARE_RELEASE_TIMEOUT")??
            .into_iter()
            .find(|v| {
                v.release_id == release_id
                    && v.entry.manifest.application_id == application_id
                    && v.entry.manifest.version == version
            })
            .ok_or("FIRMWARE_VERSION_WITHDRAWN_OR_MISSING")?;
        if current.entry.status != "active" {
            return Err("FIRMWARE_VERSION_WITHDRAWN".into());
        }
        // Loader bytes are supplied by the operator; this source never redistributes it.
        let supplied = Path::new(&loader_path)
            .canonicalize()
            .map_err(|_| "FIRMWARE_LOADER_MISSING")?;
        if current.entry.manifest.loader.sha256.to_lowercase() != lifecycle::F101_LOADER_SHA {
            return Err("FIRMWARE_LOADER_NOT_VERIFIED".into());
        }
        let mut loader = current.entry.manifest.loader.clone();
        loader.file_name = supplied
            .file_name()
            .ok_or("FIRMWARE_LOADER_MISSING")?
            .to_string_lossy()
            .into_owned();
        package::verify_file(supplied.parent().ok_or("FIRMWARE_LOADER_MISSING")?, &loader)?;
        let image = download_image(&app, &current).await?;
        let staging = lifecycle::cache_root(&app)?.join(lifecycle::id("package"));
        std::fs::create_dir(&staging).map_err(|_| "FIRMWARE_CACHE_UNAVAILABLE")?;
        std::fs::copy(image, staging.join(&current.entry.manifest.image.file_name))
            .map_err(|_| "FIRMWARE_CACHE_COPY_FAILED")?;
        std::fs::copy(
            supplied,
            staging.join(&current.entry.manifest.loader.file_name),
        )
        .map_err(|_| "FIRMWARE_CACHE_COPY_FAILED")?;
        let path = staging.join("candidate.json");
        std::fs::write(&path, lifecycle::encode(&current.entry.manifest)?)
            .map_err(|_| "FIRMWARE_CACHE_WRITE_FAILED")?;
        if lifecycle::board(&state, device_id)?.board_uid != d.board_uid {
            return Err("FIRMWARE_RELEASE_SCOPE_CHANGED".into());
        }
        let c = lifecycle::import_candidate(
            &app,
            &state,
            device_id,
            &path,
            Some(serde_json::to_value(&current).map_err(|_| "FIRMWARE_INDEX_INVALID")?),
            None,
        )?;
        ensure_current_origin(&c).await?;
        Ok(c)
    }
    .await;
    super::commands::finish_task(
        &state,
        &task_id,
        if result.is_ok() { "success" } else { "failed" },
        None,
        result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or_default(),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn withdrawal_must_be_explicit_and_well_formed() {
        assert!(withdrawal_reason(
            br#"{"schemaVersion":1,"withdrawn":true,"reason":"bad boot","at":1}"#
        )
        .is_ok());
        assert!(withdrawal_reason(
            br#"{"schemaVersion":1,"withdrawn":false,"reason":"ok","at":1}"#
        )
        .is_err());
        assert!(
            withdrawal_reason(br#"{"schemaVersion":1,"withdrawn":true,"reason":"","at":1}"#)
                .is_err()
        );
    }
    #[test]
    fn public_downloads_reject_credentials_and_non_release_hosts() {
        for u in [
            "http://github.com/a",
            "https://github.com.evil.test/a",
            "https://user:secret@github.com/a",
            "https://127.0.0.1/a",
        ] {
            assert!(validate_public_url(u).is_err());
        }
        assert!(validate_public_url("https://release-assets.githubusercontent.com/a").is_ok());
    }
    fn entry() -> FirmwareVersion {
        serde_json::from_value(json!({"manifest":{"schemaVersion":1,"applicationId":"nekobox","version":"1","boardModel":"yuzukineko","chip":"f101","route":"xip_spinor_full","flashCapacityBytes":16777216,"layout":{"bootOffset":0,"parametersOffset":131072,"applicationOffset":524288},"image":{"fileName":"app.img","sizeBytes":600000,"sha256":"a".repeat(64)},"loader":{"fileName":"fes.img","sizeBytes":10,"sha256":"b".repeat(64)},"loaderSource":"operator supplied","sourceCommit":"c".repeat(40)},"imageAssetId":5,"status":"active","withdrawalReason":"","notes":"test candidate","documentationUrl":"https://zephyr.100ask.net/docs/yuzukineko/dev/zephyr/apps/nekobox","distributionLicense":"Apache-2.0","hardware":{"model":"yuzukineko","chip":"f101","capacityBytes":16777216,"display":"1024x768","touch":"gt967"},"recipe":{"id":"nekobox-xip-v1"},"validation":{}})).unwrap()
    }
    #[test]
    fn channel_and_source_are_bound() {
        let mut index = FirmwareIndex {
            schema_version: 1,
            repository: source::REPOSITORY.into(),
            release_tag: "v1".into(),
            channel: "test".into(),
            entries: vec![entry()],
        };
        assert!(validate_index(&index, "v1", true).is_ok());
        assert!(validate_index(&index, "v1", false).is_err());
        assert!(validate_index(&index, "v2", true).is_err());
        index.repository = "dshanpi/lynx-releases".into();
        assert!(validate_index(&index, "v1", true).is_err());
    }
    #[test]
    fn withdrawal_is_not_an_available_version() {
        let mut index = FirmwareIndex {
            schema_version: 1,
            repository: source::REPOSITORY.into(),
            release_tag: "v1".into(),
            channel: "test".into(),
            entries: vec![entry()],
        };
        index.entries[0].status = "withdrawn".into();
        assert!(validate_index(&index, "v1", true).is_err());
        index.entries[0].withdrawal_reason = "failed test".into();
        assert!(validate_index(&index, "v1", true).is_ok());
    }
    #[test]
    fn duplicate_or_incompatible_entries_are_rejected() {
        let e = entry();
        let mut i = FirmwareIndex {
            schema_version: 1,
            repository: source::REPOSITORY.into(),
            release_tag: "v1".into(),
            channel: "stable".into(),
            entries: vec![e.clone(), e],
        };
        assert!(validate_index(&i, "v1", false).is_err());
        i.entries.pop();
        i.entries[0].hardware["capacityBytes"] = json!(33554432);
        assert!(validate_index(&i, "v1", false).is_err());
    }
}

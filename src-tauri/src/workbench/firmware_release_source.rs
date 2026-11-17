//! Read-only discovery from the user-selected firmware repository.
//! A discovered index is not validated firmware or permission to install.
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{storage, WorkbenchState};

pub const REPOSITORY: &str = "dshanpi/yuzukihdneko-zephyr";
pub const RELEASES_URL: &str = "https://github.com/dshanpi/yuzukihdneko-zephyr/releases";
pub const INDEX_NAME: &str = "lynx-firmware-index.json";
const API: &str = "https://api.github.com/repos/dshanpi/yuzukihdneko-zephyr/releases";
const MAX_INDEX_BYTES: u64 = 256 * 1024;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 5;

#[derive(Deserialize)]
pub(super) struct Release {
    pub(super) id: u64,
    pub(super) tag_name: String,
    pub(super) draft: bool,
    pub(super) prerelease: bool,
    pub(super) published_at: Option<String>,
    pub(super) assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(super) struct Asset {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) size: u64,
    pub(super) state: String,
    pub(super) browser_download_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseIndexCandidate {
    release_id: u64,
    tag: String,
    published_at: String,
    index_asset_id: u64,
    index_size_bytes: u64,
    index_url: String,
    release_url: String,
    validation: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareReleaseDiscovery {
    device_id: i64,
    board_uid: String,
    repository: &'static str,
    releases_url: &'static str,
    channel: &'static str,
    checked_at_ms: u64,
    stable_releases: usize,
    releases_without_index: usize,
    indices: Vec<ReleaseIndexCandidate>,
    install_authorized: bool,
}

pub(super) fn release_url(tag: &str, asset: Option<&str>) -> Result<String, String> {
    if tag.is_empty() || tag.len() > 200 || tag.chars().any(char::is_control) {
        return Err("FIRMWARE_RELEASE_DATA_INVALID".into());
    }
    let base = if asset.is_some() { "download" } else { "tag" };
    let mut url = reqwest::Url::parse(&format!("{RELEASES_URL}/{base}/"))
        .map_err(|_| "FIRMWARE_RELEASE_DATA_INVALID")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "FIRMWARE_RELEASE_DATA_INVALID")?;
        segments.pop_if_empty().push(tag);
        if let Some(name) = asset {
            segments.push(name);
        }
    }
    Ok(url.to_string())
}

fn parse_page(bytes: &[u8]) -> Result<Vec<Release>, String> {
    if bytes.len() > MAX_PAGE_BYTES {
        return Err("FIRMWARE_RELEASE_RESPONSE_TOO_LARGE".into());
    }
    let releases: Vec<Release> =
        serde_json::from_slice(bytes).map_err(|_| "FIRMWARE_RELEASE_DATA_INVALID")?;
    if releases.len() > PAGE_SIZE {
        return Err("FIRMWARE_RELEASE_DATA_INVALID".into());
    }
    Ok(releases)
}

fn collect_indices(
    releases: Vec<Release>,
) -> Result<(usize, usize, Vec<ReleaseIndexCandidate>), String> {
    let mut ids = HashSet::new();
    let mut tags = HashSet::new();
    let mut entries = Vec::new();
    let mut stable = 0;
    let mut without = 0;
    for release in releases {
        if release.draft || release.prerelease {
            continue;
        }
        if release.id == 0 || !ids.insert(release.id) || !tags.insert(release.tag_name.clone()) {
            return Err("FIRMWARE_RELEASE_DATA_INVALID".into());
        }
        let published = release
            .published_at
            .filter(|s| !s.is_empty() && s.len() <= 64)
            .ok_or("FIRMWARE_RELEASE_DATA_INVALID")?;
        let page_url = release_url(&release.tag_name, None)?;
        stable += 1;
        let assets: Vec<_> = release
            .assets
            .into_iter()
            .filter(|a| a.name == INDEX_NAME)
            .collect();
        if assets.is_empty() {
            without += 1;
            continue;
        }
        if assets.len() != 1 {
            return Err("FIRMWARE_RELEASE_INDEX_INVALID".into());
        }
        let asset = assets
            .into_iter()
            .next()
            .ok_or("FIRMWARE_RELEASE_INDEX_INVALID")?;
        let expected_url = release_url(&release.tag_name, Some(INDEX_NAME))?;
        if asset.id == 0
            || asset.state != "uploaded"
            || asset.size == 0
            || asset.size > MAX_INDEX_BYTES
            || asset.browser_download_url != expected_url
        {
            return Err("FIRMWARE_RELEASE_INDEX_INVALID".into());
        }
        entries.push(ReleaseIndexCandidate {
            release_id: release.id,
            tag: release.tag_name,
            published_at: published,
            index_asset_id: asset.id,
            index_size_bytes: asset.size,
            index_url: expected_url,
            release_url: page_url,
            validation: "not_checked",
        });
    }
    Ok((stable, without, entries))
}

fn status_error(status: reqwest::StatusCode) -> Option<&'static str> {
    match status.as_u16() {
        200 => None,
        403 | 429 => Some("FIRMWARE_RELEASE_RATE_LIMITED"),
        404 => Some("FIRMWARE_RELEASE_SOURCE_UNAVAILABLE"),
        _ => Some("FIRMWARE_RELEASE_HTTP_FAILED"),
    }
}

pub(super) async fn fetch_releases() -> Result<Vec<Release>, String> {
    let client = reqwest::Client::builder()
        .user_agent("LYNX-Firmware-Catalog/1")
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "FIRMWARE_RELEASE_NETWORK_FAILED")?;
    let mut releases = Vec::new();
    for page in 1..=MAX_PAGES {
        let mut response = client
            .get(API)
            .query(&[("per_page", PAGE_SIZE), ("page", page)])
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| "FIRMWARE_RELEASE_NETWORK_FAILED")?;
        if let Some(error) = status_error(response.status()) {
            return Err(error.into());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_PAGE_BYTES as u64)
        {
            return Err("FIRMWARE_RELEASE_RESPONSE_TOO_LARGE".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "FIRMWARE_RELEASE_NETWORK_FAILED")?
        {
            if bytes.len() + chunk.len() > MAX_PAGE_BYTES {
                return Err("FIRMWARE_RELEASE_RESPONSE_TOO_LARGE".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let items = parse_page(&bytes)?;
        let last = items.len() < PAGE_SIZE;
        releases.extend(items);
        if last {
            return Ok(releases);
        }
    }
    Err("FIRMWARE_RELEASE_CATALOG_TOO_LARGE".into())
}

fn board_uid(state: &WorkbenchState, device_id: i64) -> Result<String, String> {
    let db = state
        .db
        .lock()
        .map_err(|_| "FIRMWARE_BOARD_STATE_UNAVAILABLE")?;
    storage::list_devices(&db)
        .map_err(|_| "FIRMWARE_BOARD_STATE_UNAVAILABLE")?
        .into_iter()
        .find(|board| board.id == Some(device_id))
        .map(|board| board.board_uid)
        .ok_or_else(|| "FIRMWARE_BOARD_NOT_SAVED".into())
}

#[tauri::command]
pub async fn workbench_firmware_releases(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
) -> Result<FirmwareReleaseDiscovery, String> {
    let uid = board_uid(&state, device_id)?;
    let releases = tokio::time::timeout(Duration::from_secs(20), fetch_releases())
        .await
        .map_err(|_| "FIRMWARE_RELEASE_TIMEOUT")??;
    let (stable_releases, releases_without_index, indices) = collect_indices(releases)?;
    if board_uid(&state, device_id)? != uid {
        return Err("FIRMWARE_RELEASE_SCOPE_CHANGED".into());
    }
    let checked_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "FIRMWARE_RELEASE_CLOCK_INVALID")?
        .as_millis() as u64;
    Ok(FirmwareReleaseDiscovery {
        device_id,
        board_uid: uid,
        repository: REPOSITORY,
        releases_url: RELEASES_URL,
        channel: "stable",
        checked_at_ms,
        stable_releases,
        releases_without_index,
        indices,
        install_authorized: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn release() -> Value {
        json!({"id":1,"tag_name":"nekobox-v1.0.0","draft":false,"prerelease":false,
          "published_at":"2026-11-02T00:00:00Z","assets":[{"id":2,"name":INDEX_NAME,"size":1200,
          "state":"uploaded","browser_download_url":format!("{RELEASES_URL}/download/nekobox-v1.0.0/{INDEX_NAME}")}]})
    }
    fn collect(values: Vec<Value>) -> Result<(usize, usize, Vec<ReleaseIndexCandidate>), String> {
        collect_indices(parse_page(&serde_json::to_vec(&values).unwrap())?)
    }
    #[test]
    fn empty_is_success_not_network_error() {
        let (stable, without, indices) = collect(vec![]).unwrap();
        assert_eq!((stable, without, indices.len()), (0, 0, 0));
    }
    #[test]
    fn index_discovery_is_not_validation_or_install_permission() {
        let (stable, without, indices) = collect(vec![release()]).unwrap();
        assert_eq!((stable, without, indices.len()), (1, 0, 1));
        assert_eq!(indices[0].validation, "not_checked");
        assert_eq!(indices[0].index_asset_id, 2);
    }
    #[test]
    fn drafts_and_prereleases_do_not_enter_stable() {
        let mut draft = release();
        draft["draft"] = json!(true);
        let mut pre = release();
        pre["prerelease"] = json!(true);
        assert_eq!(collect(vec![draft, pre]).unwrap().0, 0);
    }
    #[test]
    fn ordinary_release_is_not_firmware() {
        let mut r = release();
        r["assets"] = json!([]);
        let (stable, without, indices) = collect(vec![r]).unwrap();
        assert_eq!((stable, without, indices.len()), (1, 1, 0));
    }
    #[test]
    fn reject_foreign_asset_or_tag_and_duplicates() {
        for url in ["http://github.com/x", "https://evil.test/index.json",
            "https://github.com/dshanpi/lynx-releases/releases/download/v1/lynx-firmware-index.json",
            "https://github.com/dshanpi/yuzukihdneko-zephyr/releases/download/other/lynx-firmware-index.json"] {
            let mut r=release();r["assets"][0]["browser_download_url"]=json!(url);
            assert_eq!(collect(vec![r]).unwrap_err(),"FIRMWARE_RELEASE_INDEX_INVALID");
        }
        let r = release();
        assert_eq!(
            collect(vec![r.clone(), r]).unwrap_err(),
            "FIRMWARE_RELEASE_DATA_INVALID"
        );
        let mut r = release();
        r["assets"] = json!([r["assets"][0], r["assets"][0]]);
        assert_eq!(
            collect(vec![r]).unwrap_err(),
            "FIRMWARE_RELEASE_INDEX_INVALID"
        );
    }
    #[test]
    fn reject_incomplete_oversized_or_unuploaded_index() {
        for (field, value) in [
            ("size", json!(0)),
            ("size", json!(MAX_INDEX_BYTES + 1)),
            ("id", json!(0)),
            ("state", json!("new")),
        ] {
            let mut r = release();
            r["assets"][0][field] = value;
            assert_eq!(
                collect(vec![r]).unwrap_err(),
                "FIRMWARE_RELEASE_INDEX_INVALID"
            );
        }
        assert!(parse_page(b"{}").is_err());
        assert!(parse_page(&vec![b' '; MAX_PAGE_BYTES + 1]).is_err());
        assert!(collect(vec![release(); PAGE_SIZE + 1]).is_err());
    }
    #[test]
    fn errors_and_redirects_are_not_empty_catalogs() {
        assert_eq!(status_error(reqwest::StatusCode::OK), None);
        for code in [301, 302, 401, 403, 404, 429, 500] {
            assert!(status_error(reqwest::StatusCode::from_u16(code).unwrap()).is_some());
        }
    }
    #[test]
    fn tag_paths_are_encoded_not_interpreted() {
        assert_eq!(
            release_url("neko/v1", Some(INDEX_NAME)).unwrap(),
            format!("{RELEASES_URL}/download/neko%2Fv1/{INDEX_NAME}")
        );
        assert!(release_url("", None).is_err());
        assert!(release_url("bad\n", None).is_err());
    }
}

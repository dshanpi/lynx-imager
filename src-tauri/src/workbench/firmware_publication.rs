//! Firmware publication uses the user's existing GitHub CLI identity.
//! Tokens stay in the backend; every mutation rechecks repository push permission.
use super::{
    firmware_build as build, firmware_distribution as dist, firmware_lifecycle as life,
    firmware_release_source as source, WorkbenchState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tauri::State;

static PUBLISH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
struct Github {
    api_base: String,
    upload_base: String,
    client: reqwest::Client,
    token: String,
    login: String,
}
impl Github {
    async fn connect() -> Result<Self, String> {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(["auth", "token", "--hostname", "github.com"])
            .kill_on_drop(true);
        crate::process::configure_background_command(cmd.as_std_mut());
        let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
            .await
            .map_err(|_| "FIRMWARE_GITHUB_AUTH_TIMEOUT")?
            .map_err(|_| "FIRMWARE_GITHUB_CLI_REQUIRED")?;
        if !output.status.success() || output.stdout.len() > 4096 {
            return Err("FIRMWARE_GITHUB_LOGIN_REQUIRED".into());
        }
        let token = String::from_utf8(output.stdout)
            .map_err(|_| "FIRMWARE_GITHUB_LOGIN_REQUIRED")?
            .trim()
            .to_owned();
        if token.is_empty() {
            return Err("FIRMWARE_GITHUB_LOGIN_REQUIRED".into());
        }
        let client = reqwest::Client::builder()
            .user_agent("LYNX-Firmware-Publisher/1")
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "FIRMWARE_GITHUB_UNAVAILABLE")?;
        let mut session = Self {
            api_base: "https://api.github.com/".into(),
            upload_base: "https://uploads.github.com/".into(),
            client,
            token,
            login: String::new(),
        };
        let user = session.api("GET", "user", None).await?;
        let repo = session
            .api("GET", &format!("repos/{}", source::REPOSITORY), None)
            .await?;
        if repo["full_name"] != source::REPOSITORY || repo["permissions"]["push"] != true {
            return Err("FIRMWARE_PUBLISH_PERMISSION_DENIED".into());
        }
        session.login = user["login"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("FIRMWARE_GITHUB_LOGIN_REQUIRED")?
            .into();
        Ok(session)
    }
    async fn verify_source_commit(&self, commit: &str) -> Result<(), String> {
        let actual = self
            .api(
                "GET",
                &format!("repos/{}/commits/{commit}", source::REPOSITORY),
                None,
            )
            .await
            .map_err(|e| {
                if e == "FIRMWARE_GITHUB_NOT_FOUND" {
                    "FIRMWARE_SOURCE_COMMIT_NOT_PUBLISHED".to_owned()
                } else {
                    e
                }
            })?;
        if actual["sha"] != commit {
            return Err("FIRMWARE_SOURCE_COMMIT_NOT_PUBLISHED".into());
        }
        Ok(())
    }
    async fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| "FIRMWARE_GITHUB_REQUEST_INVALID")?;
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.api_base))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "FIRMWARE_GITHUB_NETWORK_FAILED")?;
        let status = response.status();
        if status.as_u16() == 404 {
            return Err("FIRMWARE_GITHUB_NOT_FOUND".into());
        }
        if !status.is_success() {
            return Err(format!("FIRMWARE_GITHUB_HTTP_{}", status.as_u16()));
        }
        let bytes = dist::response_bytes(response, 2 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).map_err(|_| "FIRMWARE_GITHUB_RESPONSE_INVALID".into())
    }
    async fn upload(
        &self,
        release_id: u64,
        name: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<Value, String> {
        let url = format!(
            "{}repos/{}/releases/{release_id}/assets",
            self.upload_base,
            source::REPOSITORY
        );
        let r = self
            .client
            .post(url)
            .query(&[("name", name)])
            .bearer_auth(&self.token)
            .header("Content-Type", mime)
            .body(bytes)
            .send()
            .await
            .map_err(|_| "FIRMWARE_UPLOAD_FAILED")?;
        if !r.status().is_success() {
            return Err(format!("FIRMWARE_UPLOAD_HTTP_{}", r.status().as_u16()));
        }
        serde_json::from_slice(&dist::response_bytes(r, 2 * 1024 * 1024).await?)
            .map_err(|_| "FIRMWARE_UPLOAD_RESPONSE_INVALID".into())
    }
    async fn asset_bytes(&self, asset_id: u64, limit: usize) -> Result<Vec<u8>, String> {
        let r = self
            .client
            .get(format!(
                "{}repos/{}/releases/assets/{asset_id}",
                self.api_base,
                source::REPOSITORY
            ))
            .bearer_auth(&self.token)
            .header("Accept", "application/octet-stream")
            .send()
            .await
            .map_err(|_| "FIRMWARE_VERIFY_DOWNLOAD_FAILED")?;
        if r.status().is_redirection() {
            let u = r
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("FIRMWARE_ASSET_REDIRECT_INVALID")?;
            let url = reqwest::Url::parse(u).map_err(|_| "FIRMWARE_ASSET_REDIRECT_INVALID")?;
            if url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
                )
            {
                return Err("FIRMWARE_ASSET_REDIRECT_INVALID".into());
            }
            return dist::bounded(u, limit).await;
        }
        if !r.status().is_success() {
            return Err("FIRMWARE_VERIFY_DOWNLOAD_FAILED".into());
        }
        dist::response_bytes(r, limit).await
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishRequest {
    pub request_id: String,
    pub candidate_id: String,
    pub version: String,
    pub channel: String,
    pub notes: String,
    pub distribution_license: String,
    pub rights_confirmed: bool,
}
fn validate_request(r: &PublishRequest) -> Result<(), String> {
    if r.request_id.len() < 8
        || r.request_id.len() > 80
        || !r
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !matches!(r.channel.as_str(), "test" | "stable")
        || r.version.len() > 80
        || !r
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        || r.version.split('-').next().unwrap_or("").split('.').count() != 3
        || r.notes.trim().is_empty()
        || r.notes.len() > 10000
        || r.distribution_license.trim().is_empty()
        || r.distribution_license.len() > 200
        || !r.rights_confirmed
    {
        return Err("FIRMWARE_PUBLISH_FIELDS_REQUIRED".into());
    }
    if (r.channel == "stable" && r.version.contains('-'))
        || r.version.ends_with('-')
        || crate::supervisor::redact_learning_context(&r.notes) != r.notes
        || crate::supervisor::redact_learning_context(&r.distribution_license)
            != r.distribution_license
    {
        return Err("FIRMWARE_PUBLIC_TEXT_INVALID".into());
    }
    for n in r.version.split('-').next().unwrap().split('.') {
        if n.parse::<u32>().is_err() {
            return Err("FIRMWARE_VERSION_INVALID".into());
        }
    }
    Ok(())
}
fn publication_gate(
    state: &WorkbenchState,
    c: &life::Candidate,
    r: &PublishRequest,
) -> Result<Value, String> {
    validate_request(r)?;
    let checked = super::firmware_applications::inspect_candidate(
        std::path::Path::new(&c.manifest_path),
        c.device_id,
        c.board_uid.clone(),
    )?;
    if life::encode(&checked.manifest)? != life::encode(&c.manifest)?
        || c.manifest.loader.sha256 != life::F101_LOADER_SHA
    {
        return Err("FIRMWARE_CANDIDATE_CHANGED".into());
    }
    let id = c
        .build_id
        .as_deref()
        .ok_or("FIRMWARE_TRUSTED_BUILD_RECORD_REQUIRED")?;
    let record: Value = life::load(state, id, "build", &c.board_uid)?;
    let receipt = &record["receipt"];
    build::validate_receipt(receipt, id, true)?;
    let image = receipt["artifacts"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|v| v["fileName"] == c.manifest.image.file_name)
        })
        .ok_or("FIRMWARE_BUILD_ARTIFACTS_MISSING")?;
    if image["sha256"] != c.manifest.image.sha256
        || image["sizeBytes"] != c.manifest.image.size_bytes
    {
        return Err("FIRMWARE_BUILD_ARTIFACT_CHANGED".into());
    }
    if r.channel == "stable" {
        let task_id = c
            .task_ids
            .last()
            .ok_or("FIRMWARE_HARDWARE_ACCEPTANCE_REQUIRED")?;
        let t = life::task_for_candidate(state, c, task_id)?;
        let d = t
            .details
            .as_ref()
            .ok_or("FIRMWARE_PHYSICAL_EVIDENCE_REQUIRED")?;
        if t.status != "success"
            || d["physicalVerifyState"] != "success"
            || d["physicalImageSha256"] != c.manifest.image.sha256
            || d["outsideImageState"] != "unchanged"
            || d["postFlashState"] != "success"
        {
            return Err("FIRMWARE_PHYSICAL_EVIDENCE_REQUIRED".into());
        }
        let context: Value = life::load(state, task_id, "flash_context", &c.board_uid)?;
        let records = life::records(state, &c.board_uid, "acceptance")?;
        let mut seen = std::collections::HashMap::new();
        for a in records {
            if a["candidateId"] != c.id
                || a["taskId"] != *task_id
                || a["imageSha256"] != c.manifest.image.sha256
                || a["profile"] != context["profile"]
            {
                continue;
            }
            if let Some(items) = a["items"].as_array() {
                for item in items {
                    if let Some(name) = item["item"].as_str() {
                        seen.entry(name.to_owned()).or_insert(
                            item["result"].as_str().unwrap_or("not_verified").to_owned(),
                        );
                    }
                }
            }
        }
        if life::ACCEPTANCE_ITEMS
            .iter()
            .any(|i| seen.get(*i).map(String::as_str) != Some("pass"))
        {
            return Err("FIRMWARE_HARDWARE_ACCEPTANCE_REQUIRED".into());
        }
    }
    Ok(receipt.clone())
}
#[tauri::command]
pub async fn workbench_firmware_publish_check(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    request: PublishRequest,
) -> Result<Value, String> {
    let d = life::board(&state, device_id)?;
    let c: life::Candidate = life::load(&state, &request.candidate_id, "candidate", &d.board_uid)?;
    let local = publication_gate(&state, &c, &request);
    let remote = async {
        let gh = Github::connect().await?;
        gh.verify_source_commit(&c.manifest.source_commit).await?;
        Ok::<Github, String>(gh)
    }
    .await;
    Ok(
        json!({"localReady":local.is_ok(),"localError":local.err(),"permission":remote.is_ok(),"permissionError":remote.as_ref().err(),"actor":remote.as_ref().ok().map(|g|g.login.clone()),"ready":remote.is_ok()&&publication_gate(&state,&c,&request).is_ok()}),
    )
}
#[tauri::command]
pub async fn workbench_firmware_publish(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    request: PublishRequest,
    confirmed: bool,
) -> Result<Value, String> {
    let _publish_guard = PUBLISH_LOCK
        .try_lock()
        .map_err(|_| "FIRMWARE_PUBLICATION_BUSY")?;
    if !confirmed {
        return Err("FIRMWARE_CONFIRMATION_REQUIRED".into());
    }
    let d = life::board(&state, device_id)?;
    let c: life::Candidate = life::load(&state, &request.candidate_id, "candidate", &d.board_uid)?;
    let receipt = publication_gate(&state, &c, &request)?;
    let gh = Github::connect().await?;
    gh.verify_source_commit(&c.manifest.source_commit).await?;
    let id = format!("fwpublish-{}", request.request_id);
    let fingerprint = life::digest(life::encode(&request)?.as_bytes());
    let mut record = match life::load::<Value>(&state, &id, "publication", &d.board_uid) {
        Ok(r) => {
            if r["requestHash"] != fingerprint {
                return Err("FIRMWARE_PUBLICATION_INPUT_CHANGED".into());
            }
            r
        }
        Err(_) => {
            let r = json!({"id":id,"candidateId":c.id,"requestHash":fingerprint,"request":request,"version":request.version,"channel":request.channel,"status":"prepared","createdAt":life::now(),"imageSha256":c.manifest.image.sha256,"actor":gh.login,"releaseId":null});
            life::insert(&state, &id, "publication", &d.board_uid, &r)?;
            r
        }
    };
    record["request"] =
        serde_json::to_value(&request).map_err(|_| "FIRMWARE_PUBLICATION_INVALID")?;
    record["lastActor"] = json!(gh.login);
    record["lastAttemptAt"] = json!(life::now());
    life::update(&state, &id, "publication", &d.board_uid, &record)?;
    let result = publish_steps(&gh, &state, &c, &request, &receipt, &mut record).await;
    if let Err(e) = &result {
        record["status"] = json!("failed");
        record["error"] = json!(e);
        life::update(&state, &id, "publication", &d.board_uid, &record)?;
    }
    result
}
async fn publish_steps(
    gh: &Github,
    state: &WorkbenchState,
    c: &life::Candidate,
    r: &PublishRequest,
    receipt: &Value,
    record: &mut Value,
) -> Result<Value, String> {
    let id = record["id"]
        .as_str()
        .ok_or("FIRMWARE_PUBLICATION_INVALID")?
        .to_owned();
    let tag = format!("firmware-nekobox-{}", r.version);
    let release_id = if let Some(id) = record["releaseId"].as_u64() {
        id
    } else {
        let query = gh
            .api(
                "GET",
                &format!("repos/{}/releases/tags/{tag}", source::REPOSITORY),
                None,
            )
            .await;
        match query {
            Err(e) if e == "FIRMWARE_GITHUB_NOT_FOUND" => {}
            Err(e) => return Err(e),
            Ok(_) => return Err("FIRMWARE_VERSION_ALREADY_EXISTS".into()),
        }
        let release=gh.api("POST",&format!("repos/{}/releases",source::REPOSITORY),Some(json!({"tag_name":tag,"target_commitish":c.manifest.source_commit,"name":format!("NekoBox {}",r.version),"body":r.notes,"draft":true,"prerelease":r.channel=="test","make_latest":"false"}))).await?;
        let release_id = release["id"]
            .as_u64()
            .ok_or("FIRMWARE_PUBLICATION_INVALID")?;
        record["releaseId"] = json!(release_id);
        record["status"] = json!("draft");
        life::update(state, &id, "publication", &c.board_uid, record)?;
        release_id
    };
    let release = gh
        .api(
            "GET",
            &format!("repos/{}/releases/{release_id}", source::REPOSITORY),
            None,
        )
        .await?;
    if release["tag_name"] != tag {
        return Err("FIRMWARE_PUBLICATION_SCOPE_MISMATCH".into());
    }
    if release["draft"] != true {
        if record["indexSha256"].as_str().is_none() {
            return Err("FIRMWARE_PUBLISHED_VERSION_IMMUTABLE".into());
        }
        let found = dist::catalog(&r.channel).await?.into_iter().find(|v| {
            v.release_id == release_id
                && v.entry.manifest.image.sha256 == c.manifest.image.sha256
                && v.index_sha256 == record["indexSha256"].as_str().unwrap()
        });
        let found = found.ok_or("FIRMWARE_PUBLISHED_ASSETS_MISMATCH")?;
        if found.entry.status != "active" {
            return Err("FIRMWARE_VERSION_WITHDRAWN".into());
        }
        let bytes = dist::bounded(&found.image_url, c.manifest.image.size_bytes as usize).await?;
        if life::digest(&bytes) != c.manifest.image.sha256 {
            return Err("FIRMWARE_PUBLICATION_DOWNLOAD_MISMATCH".into());
        }
        record["status"] = json!("published");
        record["url"] = json!(format!("{}/tag/{tag}", source::RELEASES_URL));
        life::update(state, &id, "publication", &c.board_uid, record)?;
        return Ok(record.clone());
    }
    let bytes = tokio::fs::read(&c.image_path)
        .await
        .map_err(|_| "FIRMWARE_IMAGE_MISSING")?;
    if bytes.len() as u64 != c.manifest.image.size_bytes
        || life::digest(&bytes) != c.manifest.image.sha256
    {
        return Err("FIRMWARE_CANDIDATE_CHANGED".into());
    }
    let assets = release["assets"]
        .as_array()
        .ok_or("FIRMWARE_PUBLICATION_INVALID")?;
    let image = if let Some(a) = assets
        .iter()
        .find(|a| a["name"] == c.manifest.image.file_name)
    {
        a.clone()
    } else {
        gh.upload(
            release_id,
            &c.manifest.image.file_name,
            bytes,
            "application/octet-stream",
        )
        .await?
    };
    let image_id = image["id"].as_u64().ok_or("FIRMWARE_PUBLICATION_INVALID")?;
    let actual = gh
        .asset_bytes(image_id, c.manifest.image.size_bytes as usize)
        .await?;
    if actual.len() as u64 != c.manifest.image.size_bytes
        || life::digest(&actual) != c.manifest.image.sha256
    {
        return Err("FIRMWARE_UPLOADED_IMAGE_MISMATCH".into());
    }
    let mut manifest = c.manifest.clone();
    manifest.version = r.version.clone();
    let index = dist::FirmwareIndex {
        schema_version: 1,
        repository: source::REPOSITORY.into(),
        release_tag: tag.clone(),
        channel: r.channel.clone(),
        entries: vec![dist::FirmwareVersion {
            manifest,
            image_asset_id: image_id,
            status: "active".into(),
            withdrawal_reason: String::new(),
            notes: r.notes.clone(),
            documentation_url: "https://zephyr.100ask.net/docs/yuzukineko/dev/zephyr/apps/nekobox"
                .into(),
            distribution_license: r.distribution_license.clone(),
            hardware: json!({"model":"yuzukineko","chip":"f101","capacityBytes":16777216,"display":"1024x768","touch":"gt967"}),
            recipe: json!({"id":"nekobox-xip-v1","inputs":receipt["inputs"],"toolchain":receipt["toolchain"],"artifacts":receipt["artifacts"]}),
            validation: json!({"build":"passed","hardware":if r.channel=="stable"{"passed"}else{"not_verified"},"imageSha256":c.manifest.image.sha256}),
        }],
    };
    dist::validate_index(&index, &tag, r.channel == "test")?;
    let encoded = life::encode(&index)?.into_bytes();
    let index_sha = life::digest(&encoded);
    let asset = if let Some(a) = assets.iter().find(|a| a["name"] == source::INDEX_NAME) {
        a.clone()
    } else {
        gh.upload(
            release_id,
            source::INDEX_NAME,
            encoded.clone(),
            "application/json",
        )
        .await?
    };
    let actual = gh
        .asset_bytes(
            asset["id"].as_u64().ok_or("FIRMWARE_PUBLICATION_INVALID")?,
            256 * 1024,
        )
        .await?;
    if life::digest(&actual) != index_sha {
        return Err("FIRMWARE_UPLOADED_INDEX_MISMATCH".into());
    }
    record["status"] = json!("verified_draft");
    record["indexSha256"] = json!(index_sha);
    life::update(state, &id, "publication", &c.board_uid, record)?;
    publication_gate(state, c, r)?;
    if Github::connect().await?.login != gh.login {
        return Err("FIRMWARE_PUBLISH_IDENTITY_CHANGED".into());
    }
    gh.api("PATCH",&format!("repos/{}/releases/{release_id}",source::REPOSITORY),Some(json!({"draft":false,"prerelease":r.channel=="test","make_latest":if r.channel=="stable"{"true"}else{"false"}}))).await?;
    record["status"] = json!("verification_pending");
    life::update(state, &id, "publication", &c.board_uid, record)?;
    let verified = dist::catalog(&r.channel)
        .await?
        .into_iter()
        .find(|v| v.release_id == release_id && v.index_sha256 == index_sha)
        .ok_or("FIRMWARE_PUBLICATION_VERIFY_PENDING")?;
    let downloaded =
        dist::bounded(&verified.image_url, c.manifest.image.size_bytes as usize).await?;
    if life::digest(&downloaded) != c.manifest.image.sha256 {
        return Err("FIRMWARE_PUBLICATION_DOWNLOAD_MISMATCH".into());
    }
    record["status"] = json!("published");
    record["url"] = json!(format!("{}/tag/{tag}", source::RELEASES_URL));
    record["publishedAt"] = json!(life::now());
    life::update(state, &id, "publication", &c.board_uid, record)?;
    Ok(record.clone())
}
#[tauri::command]
pub async fn workbench_firmware_release_action(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    publication_id: String,
    action: String,
    reason: String,
    confirmed: bool,
) -> Result<Value, String> {
    let _publish_guard = PUBLISH_LOCK
        .try_lock()
        .map_err(|_| "FIRMWARE_PUBLICATION_BUSY")?;
    if crate::supervisor::redact_learning_context(&reason) != reason {
        return Err("FIRMWARE_PUBLIC_TEXT_INVALID".into());
    }
    if !confirmed
        || !matches!(action.as_str(), "withdraw" | "recommend")
        || reason.trim().is_empty()
        || reason.len() > 2000
    {
        return Err("FIRMWARE_RELEASE_ACTION_INVALID".into());
    }
    let d = life::board(&state, device_id)?;
    let mut p: Value = life::load(&state, &publication_id, "publication", &d.board_uid)?;
    let gh = Github::connect().await?;
    let release_id = p["releaseId"]
        .as_u64()
        .ok_or("FIRMWARE_PUBLICATION_NOT_PUBLISHED")?;
    let release = gh
        .api(
            "GET",
            &format!("repos/{}/releases/{release_id}", source::REPOSITORY),
            None,
        )
        .await?;
    if release["draft"] != false {
        return Err("FIRMWARE_PUBLICATION_NOT_PUBLISHED".into());
    }
    let event = json!({"id":life::id("fwrelease-action"),"action":action,"releaseId":release_id,"actor":gh.login,"reason":crate::supervisor::redact_learning_context(&reason),"at":life::now()});
    life::insert(
        &state,
        event["id"].as_str().unwrap(),
        "publication_event",
        &d.board_uid,
        &event,
    )?;
    if action == "withdraw" {
        let existing = release["assets"]
            .as_array()
            .ok_or("FIRMWARE_PUBLICATION_INVALID")?
            .iter()
            .any(|a| a["name"] == "lynx-withdrawal.json");
        if !existing {
            gh.upload(release_id,"lynx-withdrawal.json",life::encode(&json!({"schemaVersion":1,"withdrawn":true,"reason":event["reason"],"at":event["at"]}))?.into_bytes(),"application/json").await?;
        }
        p["status"] = json!("withdrawn");
    } else {
        if release["prerelease"] == true
            || p["status"] != "published"
            || release["assets"]
                .as_array()
                .is_some_and(|a| a.iter().any(|a| a["name"] == "lynx-withdrawal.json"))
        {
            return Err("FIRMWARE_RECOMMENDATION_REJECTED".into());
        }
        gh.api(
            "PATCH",
            &format!("repos/{}/releases/{release_id}", source::REPOSITORY),
            Some(json!({"make_latest":"true"})),
        )
        .await?;
        p["recommendedAt"] = json!(life::now());
    }
    life::update(&state, &publication_id, "publication", &d.board_uid, &p)?;
    Ok(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    // Local transport simulation only. No GitHub credentials, public mutation or hardware.
    fn mock_server(
        responses: Vec<(u16, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = vec![];
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut b = [0; 8192];
                let (header_end, size) = loop {
                    let n = stream.read(&mut b).unwrap();
                    assert!(n > 0);
                    raw.extend_from_slice(&b[..n]);
                    if let Some(i) = raw.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&raw[..i]);
                        let len = header
                            .lines()
                            .find_map(|s| {
                                s.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        requests.push(header.lines().next().unwrap().to_owned());
                        break (i + 4, len);
                    }
                    assert!(raw.len() < 65536);
                };
                while raw.len() < header_end + size {
                    let n = stream.read(&mut b).unwrap();
                    assert!(n > 0);
                    raw.extend_from_slice(&b[..n]);
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (url, handle)
    }
    #[tokio::test]
    async fn partial_index_upload_keeps_draft_and_retry_reuses_image_without_overwrite() {
        let state = WorkbenchState::open(std::path::Path::new(":memory:")).unwrap();
        let dir = std::env::temp_dir().join(life::id("lynx-fw-pub-test"));
        std::fs::create_dir(&dir).unwrap();
        let bytes = vec![90u8; 600 * 1024];
        let path = dir.join("nekobox-full.img");
        std::fs::write(&path, &bytes).unwrap();
        let manifest:super::super::firmware_applications::FirmwareCandidateManifest=serde_json::from_value(json!({"schemaVersion":1,"applicationId":"nekobox","version":"build-test","boardModel":"yuzukineko","chip":"f101","route":"xip_spinor_full","flashCapacityBytes":16777216,"layout":{"bootOffset":0,"parametersOffset":131072,"applicationOffset":524288},"image":{"fileName":"nekobox-full.img","sizeBytes":bytes.len(),"sha256":life::digest(&bytes)},"loader":{"fileName":"loader.img","sizeBytes":4961280,"sha256":life::F101_LOADER_SHA},"loaderSource":"operator supplied; not distributed","sourceCommit":"a".repeat(40)})).unwrap();
        let c = life::Candidate {
            id: "candidate-test".into(),
            device_id: 1,
            board_uid: "board-test".into(),
            manifest,
            manifest_path: String::new(),
            image_path: path.to_string_lossy().into_owned(),
            loader_path: String::new(),
            created_at: 1,
            origin: None,
            build_id: None,
            task_ids: vec![],
        };
        let request = PublishRequest {
            request_id: "request-test".into(),
            candidate_id: c.id.clone(),
            version: "0.1.0-test.1".into(),
            channel: "test".into(),
            notes: "Simulated transport failure".into(),
            distribution_license: "not-for-distribution".into(),
            rights_confirmed: true,
        };
        let release =
            json!({"id":123,"tag_name":"firmware-nekobox-0.1.0-test.1","draft":true,"assets":[]});
        let reused = json!({"id":123,"tag_name":"firmware-nekobox-0.1.0-test.1","draft":true,"assets":[{"id":10,"name":"nekobox-full.img"}]});
        let j = |s: Value| serde_json::to_vec(&s).unwrap();
        let (base, server) = mock_server(vec![
            (404, j(json!({}))),
            (200, j(json!({"id":123}))),
            (200, j(release)),
            (201, j(json!({"id":10}))),
            (200, bytes.clone()),
            (500, j(json!({}))),
            (200, j(reused)),
            (200, bytes),
            (500, j(json!({}))),
        ]);
        let gh = Github {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            token: "isolated-synthetic-token".into(),
            login: "test".into(),
            api_base: base.clone(),
            upload_base: base,
        };
        let mut record = json!({"id":"publication-test","releaseId":null,"status":"prepared","request":request,"requestHash":life::digest(life::encode(&request).unwrap().as_bytes())});
        life::insert(
            &state,
            "publication-test",
            "publication",
            &c.board_uid,
            &record,
        )
        .unwrap();
        for _ in 0..2 {
            record = life::load(&state, "publication-test", "publication", &c.board_uid).unwrap();
            let request: PublishRequest =
                serde_json::from_value(record["request"].clone()).unwrap();
            assert_eq!(
                record["requestHash"],
                life::digest(life::encode(&request).unwrap().as_bytes())
            );
            assert_eq!(
                publish_steps(&gh, &state, &c, &request, &json!({}), &mut record)
                    .await
                    .unwrap_err(),
                "FIRMWARE_UPLOAD_HTTP_500"
            );
            assert_eq!(record["status"], "draft");
            assert_eq!(record["releaseId"], 123);
        }
        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /repos/") && r.contains("/releases HTTP"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.contains("name=nekobox-full.img"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.contains("name=lynx-firmware-index.json"))
                .count(),
            2
        );
        assert!(!requests
            .iter()
            .any(|r| r.starts_with("PATCH ") || r.starts_with("DELETE ")));
    }
    #[tokio::test]
    async fn bounded_response_rejects_oversized_and_accepts_exact_length() {
        let (base, server) = mock_server(vec![(200, vec![0; 4096])]);
        let response = reqwest::get(&base).await.unwrap();
        assert!(dist::response_bytes(response, 1024).await.is_err());
        server.join().unwrap();
        let (base, server) = mock_server(vec![(200, b"exact bytes".to_vec())]);
        assert_eq!(
            dist::response_bytes(reqwest::get(&base).await.unwrap(), 11)
                .await
                .unwrap(),
            b"exact bytes"
        );
        server.join().unwrap();
    }

    #[test]
    fn publish_requires_complete_fields_and_rights() {
        let mut r = PublishRequest {
            request_id: "abcdefgh".into(),
            candidate_id: "c".into(),
            version: "1.2.3".into(),
            channel: "test".into(),
            notes: "test".into(),
            distribution_license: "Apache-2.0".into(),
            rights_confirmed: true,
        };
        assert!(validate_request(&r).is_ok());
        r.rights_confirmed = false;
        assert!(validate_request(&r).is_err());
        r.rights_confirmed = true;
        r.version = "../../wrong".into();
        assert!(validate_request(&r).is_err());
    }
}

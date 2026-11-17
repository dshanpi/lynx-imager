//! Learn v2 negotiation. Advertise only implemented, independently gated capabilities.
use std::{collections::HashSet, sync::Arc, time::Duration};

use reqwest::{redirect::Policy, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::auth::state::AuthState;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const CLIENT_CAPABILITIES: &[&str] = manifest::READ_CAPABILITIES;

pub mod board;
pub mod catalog;
pub mod courses;
pub mod environment;
pub mod execution;
pub mod manifest;
pub mod preview;
pub mod progress;
pub mod review;
pub mod tutor;
mod write_errors;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnError {
    code: i32,
    name: &'static str,
    message: &'static str,
    machine_code: &'static str,
    retryable: bool,
}

fn error(code: &'static str, message: &'static str, retryable: bool) -> LearnError {
    LearnError {
        code: -1,
        name: "LearnError",
        message,
        machine_code: code,
        retryable,
    }
}
fn schema_error() -> LearnError {
    error(
        "LEARN_SCHEMA_MISMATCH",
        "官网课程接口格式不兼容，请联系管理员核对协议",
        false,
    )
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnLimits {
    pub max_event_batch_items: u32,
    pub max_event_batch_bytes: u32,
    pub max_json_bytes: u32,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnCapabilities {
    pub protocols: Vec<String>,
    pub manifest_formats: Vec<String>,
    pub action_catalog_versions: Vec<String>,
    pub min_client_requirements: Vec<String>,
    pub limits: LearnLimits,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    request_id: String,
    server_time: String,
}
#[derive(Deserialize)]
struct Envelope {
    data: LearnCapabilities,
    meta: Meta,
}
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    NotDeployed,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnCapabilitiesResult {
    pub availability: Availability,
    pub server: Option<LearnCapabilities>,
    pub client_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
}

fn not_deployed() -> LearnCapabilitiesResult {
    LearnCapabilitiesResult {
        availability: Availability::NotDeployed,
        server: None,
        client_capabilities: CLIENT_CAPABILITIES.iter().map(|s| (*s).into()).collect(),
        missing_capabilities: vec![],
    }
}
fn valid_atom(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
}
fn valid_list(items: &[String]) -> bool {
    items.len() <= 64
        && items.iter().all(|s| valid_atom(s))
        && items.iter().collect::<HashSet<_>>().len() == items.len()
}

fn parse_capabilities(bytes: &[u8]) -> Result<LearnCapabilitiesResult, LearnError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(error(
            "LEARN_RESPONSE_TOO_LARGE",
            "官网课程接口响应超出限制",
            false,
        ));
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| schema_error())?;
    let c = envelope.data;
    if ![
        &c.protocols,
        &c.manifest_formats,
        &c.action_catalog_versions,
        &c.min_client_requirements,
    ]
    .iter()
    .all(|list| valid_list(list))
        || !valid_atom(&envelope.meta.request_id)
        || envelope.meta.server_time.len() > 64
        || envelope.meta.server_time.is_empty()
        || !(1..=100).contains(&c.limits.max_event_batch_items)
        || !(1..=524_288).contains(&c.limits.max_event_batch_bytes)
        || !(1..=1_048_576).contains(&c.limits.max_json_bytes)
    {
        return Err(schema_error());
    }
    if !c.protocols.iter().any(|p| p == "learn.v2") {
        return Err(error(
            "LEARN_PROTOCOL_UNSUPPORTED",
            "官网未声明支持当前学习接口协议",
            false,
        ));
    }
    let missing = c
        .min_client_requirements
        .iter()
        .filter(|item| !CLIENT_CAPABILITIES.contains(&item.as_str()))
        .cloned()
        .collect();
    Ok(LearnCapabilitiesResult {
        availability: Availability::Available,
        client_capabilities: CLIENT_CAPABILITIES.iter().map(|s| (*s).into()).collect(),
        server: Some(c),
        missing_capabilities: missing,
    })
}

fn capabilities_url(base: &str, allow_loopback: bool) -> Result<Url, LearnError> {
    let invalid = || {
        error(
            "LEARN_ORIGIN_UNTRUSTED",
            "课程服务地址不在受信官网范围内",
            false,
        )
    };
    let mut url = Url::parse(base).map_err(|_| invalid())?;
    let trusted = url.scheme() == "https"
        && matches!(url.host_str(), Some("100ask.net" | "www.100ask.net"))
        && url.port_or_known_default() == Some(443);
    let local = allow_loopback
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if !(trusted || local)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(invalid());
    }
    url.set_path("/api/v2/lynx/learn/capabilities");
    url.query_pairs_mut()
        .append_pair("clientVersion", env!("CARGO_PKG_VERSION"))
        .append_pair("platform", std::env::consts::OS);
    Ok(url)
}

fn status_error(status: StatusCode) -> LearnError {
    match status.as_u16() {
        401 => error("LEARN_AUTH_REQUIRED", "登录已失效，请重新登录后检查", false),
        403 => error("LEARN_FORBIDDEN", "当前账号无权访问新版课程接口", false),
        409 => error(
            "LEARN_PREVIEW_STALE",
            "草稿或请求已变化，请重新打开试学",
            false,
        ),
        410 => error(
            "LEARN_PREVIEW_EXPIRED",
            "试学邀请已过期，请在官网重新生成",
            false,
        ),
        413 => error(
            "LEARN_PREVIEW_LIMIT",
            "课程内容超过当前协议限制，请拆分预览",
            false,
        ),
        429 => error("LEARN_RATE_LIMITED", "官网请求频率受限，请稍后重试", true),
        300..=399 => error(
            "LEARN_REDIRECT_REJECTED",
            "官网课程接口发生重定向，请管理员检查部署",
            false,
        ),
        500..=599 => error(
            "LEARN_SERVICE_UNAVAILABLE",
            "官网课程服务暂不可用，请稍后重试",
            true,
        ),
        _ => error("LEARN_HTTP_ERROR", "官网课程接口返回异常状态", false),
    }
}

async fn request_capabilities(
    url: Url,
    token: &str,
) -> Result<LearnCapabilitiesResult, LearnError> {
    match get_response(url, token, MAX_RESPONSE_BYTES, true).await? {
        Some(bytes) => parse_capabilities(&bytes),
        None => Ok(not_deployed()),
    }
}

async fn get_response(
    url: Url,
    token: &str,
    max_bytes: usize,
    allow_missing: bool,
) -> Result<Option<Vec<u8>>, LearnError> {
    response_bytes(url, token, max_bytes, allow_missing, None).await
}
async fn post_response(
    url: Url,
    token: &str,
    body: &serde_json::Value,
    request_id: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, LearnError> {
    response_bytes(url, token, max_bytes, false, Some((body, request_id)))
        .await?
        .ok_or_else(schema_error)
}
async fn response_bytes(
    url: Url,
    token: &str,
    max_bytes: usize,
    allow_missing: bool,
    post: Option<(&serde_json::Value, &str)>,
) -> Result<Option<Vec<u8>>, LearnError> {
    // Separate no-redirect client: an auth redirect must not become an HTML success or leak tokens.
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| error("LEARN_CLIENT_UNAVAILABLE", "课程连接初始化失败", true))?;
    let request = if let Some((body, request_id)) = post {
        client
            .post(url)
            .json(body)
            .header("Idempotency-Key", request_id)
    } else {
        client.get(url)
    };
    let request = if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    };
    let mut response = request
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| {
            error(
                "LEARN_NETWORK_UNAVAILABLE",
                "无法连接官网课程服务，请检查网络",
                true,
            )
        })?;
    if allow_missing && matches!(response.status().as_u16(), 404 | 501) {
        return Ok(None);
    }
    if response.status() == StatusCode::NOT_FOUND {
        return Err(error(
            "LEARN_NOT_FOUND",
            "课程或课时已不可访问，请重新同步课程",
            false,
        ));
    }
    if response.status() != StatusCode::OK
        && !(post.is_some() && matches!(response.status().as_u16(), 201 | 202))
    {
        if post.is_some() && response.status() == StatusCode::CONFLICT {
            return Err(write_errors::conflict(response).await);
        }
        return Err(status_error(response.status()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !content_type.eq_ignore_ascii_case("application/json") {
        return Err(schema_error());
    }
    if response
        .content_length()
        .is_some_and(|n| n > max_bytes as u64)
    {
        return Err(error(
            "LEARN_RESPONSE_TOO_LARGE",
            "官网课程接口响应超出限制",
            false,
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        error(
            "LEARN_RESPONSE_INTERRUPTED",
            "官网响应中断，请稍后重试",
            true,
        )
    })? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(error(
                "LEARN_RESPONSE_TOO_LARGE",
                "官网课程接口响应超出限制",
                false,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

#[tauri::command]
pub async fn learn_get_capabilities(
    auth: State<'_, Arc<AuthState>>,
) -> Result<LearnCapabilitiesResult, LearnError> {
    let base = std::env::var("LYNX_LEARNING_API_BASE_URL")
        .unwrap_or_else(|_| "https://www.100ask.net".into());
    let url = capabilities_url(&base, cfg!(debug_assertions))?;
    let token = auth.get_access_token().await.map_err(|_| {
        error(
            "LEARN_AUTH_REQUIRED",
            "请先登录官网账号，再检查课程连接",
            false,
        )
    })?;
    request_capabilities(url, &token).await
}

#[cfg(test)]
mod tests;

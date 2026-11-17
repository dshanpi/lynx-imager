//! Bounded API writes use journal bytes verbatim. Signed upload URLs stay in memory.
use super::*;
use journal::Receipt;
use reqwest::{redirect::Policy, Url};
use std::time::Duration;

const MAX_RESPONSE: usize = 64 * 1024;
#[derive(Debug, Clone)]
pub(super) struct Fault {
    pub code: &'static str,
    pub retryable: bool,
}
fn fault(code: &'static str, retryable: bool) -> Fault {
    Fault { code, retryable }
}
pub(super) fn unknown() -> Fault {
    fault("LEARN_WRITE_UNCONFIRMED", true)
}

// Deliberately no Serialize or Debug: the signed URL may contain credentials.
pub(super) struct UploadGrant {
    pub url: Url,
    pub content_type: String,
    pub upload_id: String,
    pub expires_at: chrono::DateTime<chrono::FixedOffset>,
}
pub(super) struct Delivery {
    pub receipt: Receipt,
    pub grant: Option<UploadGrant>,
}

fn endpoint(base: &str, route: &Route) -> Result<Url, Fault> {
    use super::super::super::courses;
    let url = match route {
        Route::Session => courses::url(base, &["sessions"]),
        Route::Events { attempt_id } => {
            let mut url = courses::url(base, &["attempts", attempt_id])
                .map_err(|_| fault("LEARN_ORIGIN_UNTRUSTED", false))?;
            url.path_segments_mut()
                .map_err(|_| unknown())?
                .push("events:batch");
            Ok(url)
        }
        Route::Evidence { attempt_id } => courses::url(base, &["attempts", attempt_id, "evidence"]),
        Route::Grant { evidence_id } => {
            courses::url(base, &["evidence", evidence_id, "upload-grants"])
        }
        Route::Completion { evidence_id } => {
            courses::url(base, &["evidence", evidence_id, "upload-completion"])
        }
        Route::Upload { .. } => return Err(fault("LEARN_SYNC_ROUTE_INVALID", false)),
    };
    url.map_err(|_| fault("LEARN_ORIGIN_UNTRUSTED", false))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantWire {
    upload_id: String,
    url: String,
    expires_at: String,
    headers: std::collections::BTreeMap<String, String>,
}
fn parse_grant(request: &Request, value: Value) -> Result<Delivery, Fault> {
    let wire: GrantWire = serde_json::from_value(value).map_err(|_| unknown())?;
    let Route::Grant { evidence_id } = &request.route else {
        return Err(unknown());
    };
    let request_body: Value = serde_json::from_str(&request.body).map_err(|_| unknown())?;
    let url = Url::parse(&wire.url).map_err(|_| unknown())?;
    let expires = chrono::DateTime::parse_from_rfc3339(&wire.expires_at).map_err(|_| unknown())?;
    if !manifest::valid_id(&wire.upload_id)
        || wire.url.len() > 8192
        || url.scheme() != "https"
        || url.domain().is_none()
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_none_or(str::is_empty)
        || url.path() != format!("/learn/pending/{evidence_id}/{}", wire.upload_id)
        || wire.expires_at.len() > 64
        || expires > chrono::Utc::now() + chrono::Duration::minutes(15)
        || wire.headers.len() != 2
    {
        return Err(fault("LEARN_UPLOAD_GRANT_INVALID", false));
    }
    let mut content_type = None;
    let mut acl = false;
    for (name, value) in wire.headers {
        match name.to_ascii_lowercase().as_str() {
            "content-type"
                if content_type.is_none()
                    && request_body["contentType"].as_str() == Some(&value) =>
            {
                content_type = Some(value)
            }
            "x-oss-object-acl" if !acl && value == "private" => acl = true,
            _ => return Err(fault("LEARN_UPLOAD_GRANT_INVALID", false)),
        }
    }
    if !acl {
        return Err(fault("LEARN_UPLOAD_GRANT_INVALID", false));
    }
    let grant = UploadGrant {
        url,
        content_type: content_type.ok_or_else(unknown)?,
        upload_id: wire.upload_id.clone(),
        expires_at: expires,
    };
    Ok(Delivery {
        receipt: Receipt::Grant {
            upload_id: wire.upload_id,
            expires_at: wire.expires_at,
        },
        grant: Some(grant),
    })
}
fn parse_data(request: &Request, mut value: Value) -> Result<Delivery, Fault> {
    if matches!(request.route, Route::Grant { .. }) {
        return parse_grant(request, value);
    }
    let kind = match request.route {
        Route::Session => "session",
        Route::Events { .. } => "events",
        Route::Evidence { .. } => "evidence",
        Route::Completion { .. } => "completion",
        _ => return Err(unknown()),
    };
    let object = value.as_object_mut().ok_or_else(unknown)?;
    if object.insert("kind".into(), json!(kind)).is_some() {
        return Err(unknown());
    }
    let receipt: Receipt = serde_json::from_value(value).map_err(|_| unknown())?;
    Ok(Delivery {
        receipt,
        grant: None,
    })
}
#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorWire,
}
#[derive(Deserialize)]
struct ErrorWire {
    code: String,
    retryable: bool,
}
fn rejected(request: &Request, status: u16, bytes: &[u8]) -> Result<Delivery, Fault> {
    let envelope: ErrorEnvelope = serde_json::from_slice(bytes).map_err(|_| unknown())?;
    let error = envelope.error;
    if status == 409 {
        match (error.code.as_str(), error.retryable) {
            ("LEARN_OPERATION_IN_PROGRESS", true) => {
                return Err(fault("LEARN_OPERATION_IN_PROGRESS", true))
            }
            ("LEARN_UPLOAD_EXPIRED" | "LEARN_UPLOAD_INCOMPLETE", false) => {
                if matches!(request.route, Route::Completion { .. }) {
                    let body: Value = serde_json::from_str(&request.body).map_err(|_| unknown())?;
                    let id = body["uploadId"]
                        .as_str()
                        .filter(|v| manifest::valid_id(v))
                        .ok_or_else(unknown)?;
                    let receipt = if error.code == "LEARN_UPLOAD_EXPIRED" {
                        Receipt::GrantExpired {
                            upload_id: id.into(),
                        }
                    } else {
                        Receipt::UploadIncomplete {
                            upload_id: id.into(),
                        }
                    };
                    return Ok(Delivery {
                        receipt,
                        grant: None,
                    });
                }
            }
            (
                "IDEMPOTENCY_CONFLICT"
                | "LEARN_REVISION_CONFLICT"
                | "LEARN_SEQUENCE_GAP"
                | "LEARN_WRITER_CONFLICT"
                | "LEARN_EVENT_CONFLICT"
                | "LEARN_RUN_CONFLICT"
                | "LEARN_RUN_TERMINAL"
                | "LEARN_INVALID_RELEASE"
                | "LEARN_ENVIRONMENT_CONFLICT"
                | "LEARN_EVIDENCE_STALE"
                | "LEARN_ATTEMPT_TERMINAL"
                | "LEARN_ATTEMPT_CONFLICT"
                | "LEARN_UPLOAD_CONFLICT",
                false,
            ) => return Err(fault("LEARN_SYNC_CONFLICT", false)),
            _ => {}
        }
    }
    if status == 422 && !error.retryable {
        return Err(fault("LEARN_SYNC_REJECTED", false));
    }
    if status >= 500 {
        return Err(fault("LEARN_SERVICE_UNAVAILABLE", true));
    }
    Err(unknown())
}
pub(super) async fn post(base: &str, token: &str, request: &Request) -> Result<Delivery, Fault> {
    request
        .validate()
        .map_err(|_| fault("LEARN_SYNC_REQUEST_INVALID", false))?;
    if token.is_empty() {
        return Err(fault("LEARN_AUTH_REQUIRED", false));
    }
    let url = endpoint(base, &request.route)?;
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| unknown())?;
    let mut response = client
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &request.key)
        .body(request.body.clone())
        .send()
        .await
        .map_err(|_| unknown())?;
    let status = response.status().as_u16();
    match status {
        401 => return Err(fault("LEARN_AUTH_REQUIRED", false)),
        403 => return Err(fault("LEARN_FORBIDDEN", false)),
        300..=399 => return Err(fault("LEARN_REDIRECT_REJECTED", false)),
        429 => return Err(fault("LEARN_RATE_LIMITED", true)),
        _ => {}
    }
    let json = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/json");
    if !json
        || response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err(unknown());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err(unknown());
        }
        bytes.extend_from_slice(&chunk);
    }
    if ![200, 201, 202].contains(&status) {
        return rejected(request, status, &bytes);
    }
    let mut body: Value = serde_json::from_slice(&bytes).map_err(|_| unknown())?;
    let data = body
        .as_object_mut()
        .and_then(|o| o.remove("data"))
        .ok_or_else(unknown)?;
    parse_data(request, data)
}

#[cfg(test)]
mod tests;

use super::*;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../docs/contracts/learn-v2/capabilities.response.json"
    ))
    .unwrap()
}
fn parse(v: &Value) -> Result<LearnCapabilitiesResult, LearnError> {
    parse_capabilities(&serde_json::to_vec(v).unwrap())
}

#[test]
fn missing_runtime_capabilities_are_not_advertised() {
    let mut v = fixture();
    v["data"]["minClientRequirements"] = json!(["learn.capabilities.v1", "learn.ssh-workspace.v1"]);
    let r = parse(&v).unwrap();
    assert_eq!(r.availability, Availability::Available);
    assert_eq!(r.missing_capabilities, vec!["learn.ssh-workspace.v1"]);
    assert_eq!(
        r.client_capabilities,
        vec![
            "learn.capabilities.v1",
            "learn.course-manifest.v1",
            "learn.course-manifest.v2",
            "learn.course-read.v1",
            "learn.project-task.v1",
            "learn.board-observation.v1"
        ]
    );
}
#[test]
fn malformed_envelopes_limits_and_duplicate_features_are_rejected() {
    for mutation in [
        json!(null),
        json!({"data":{}}),
        json!({"data":fixture()["data"]}),
    ] {
        assert!(parse(&mutation).is_err());
    }
    for key in ["maxEventBatchItems", "maxEventBatchBytes", "maxJsonBytes"] {
        let mut v = fixture();
        v["data"]["limits"][key] = json!(0);
        assert!(parse(&v).is_err());
        v["data"]["limits"][key] = json!(2_000_000);
        assert!(parse(&v).is_err());
    }
    for list in [
        json!(["learn.v2", "learn.v2"]),
        json!(["bearer secret"]),
        json!(vec!["x"; 65]),
    ] {
        let mut v = fixture();
        v["data"]["protocols"] = list;
        assert!(parse(&v).is_err());
    }
    assert!(parse_capabilities(&vec![b' '; MAX_RESPONSE_BYTES + 1]).is_err());
    assert!(parse_capabilities(b"<html>login</html>").is_err());
}
#[test]
fn legacy_and_unknown_protocol_are_not_v2() {
    let mut v = fixture();
    v["data"]["protocols"] = json!(["learn.v3"]);
    assert_eq!(
        parse(&v).unwrap_err().machine_code,
        "LEARN_PROTOCOL_UNSUPPORTED"
    );
}
#[test]
fn url_origin_is_validated_before_any_bearer_request() {
    for bad in [
        "https://evil.example",
        "https://100ask.net.evil.example",
        "https://user:secret@100ask.net",
        "https://100ask.net/path",
        "https://100ask.net/?token=secret",
        "https://100ask.net/#fragment",
        "https://100ask.net:444",
        "http://100ask.net",
        "http://127.0.0.1:5219",
    ] {
        assert!(capabilities_url(bad, false).is_err(), "{bad}");
    }
    assert!(capabilities_url("http://127.0.0.1:5219", true).is_ok());
    let u = capabilities_url("https://www.100ask.net", false).unwrap();
    assert_eq!(u.path(), "/api/v2/lynx/learn/capabilities");
    assert!(u
        .query_pairs()
        .any(|(k, v)| k == "clientVersion" && v == env!("CARGO_PKG_VERSION")));
}

async fn replay(
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    extra: &str,
) -> (Result<LearnCapabilitiesResult, LearnError>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let headers=format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",body.len());
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..n]);
        }
        stream.write_all(headers.as_bytes()).unwrap();
        let _ = stream.write_all(&body);
        String::from_utf8(request).unwrap()
    });
    let url = capabilities_url(&format!("http://{address}"), true).unwrap();
    let result = request_capabilities(url, "mock-only-token").await;
    (result, task.join().unwrap())
}
#[tokio::test]
async fn authenticated_http_contract_and_no_redirect() {
    let (r, request) = replay(
        200,
        "application/json; charset=utf-8",
        serde_json::to_vec(&fixture()).unwrap(),
        "",
    )
    .await;
    assert_eq!(r.unwrap().availability, Availability::Available);
    assert!(request.starts_with("GET /api/v2/lynx/learn/capabilities?"));
    assert!(request
        .to_lowercase()
        .contains("authorization: bearer mock-only-token"));
    let (r, _) = replay(
        302,
        "text/html",
        b"secret".to_vec(),
        "Location: http://127.0.0.1:1/stolen\r\n",
    )
    .await;
    assert_eq!(r.unwrap_err().machine_code, "LEARN_REDIRECT_REJECTED");
}
#[tokio::test]
async fn errors_are_not_empty_success_or_credential_echo() {
    for status in [404, 501] {
        assert_eq!(
            replay(status, "text/html", vec![], "")
                .await
                .0
                .unwrap()
                .availability,
            Availability::NotDeployed
        );
    }
    for (status, code) in [
        (401, "LEARN_AUTH_REQUIRED"),
        (403, "LEARN_FORBIDDEN"),
        (429, "LEARN_RATE_LIMITED"),
        (500, "LEARN_SERVICE_UNAVAILABLE"),
        (204, "LEARN_HTTP_ERROR"),
    ] {
        let e = replay(status, "application/json", b"secret-canary".to_vec(), "")
            .await
            .0
            .unwrap_err();
        assert_eq!(e.machine_code, code);
        assert!(!serde_json::to_string(&e).unwrap().contains("secret-canary"));
    }
    assert!(replay(200, "text/html", b"login".to_vec(), "")
        .await
        .0
        .is_err());
    assert_eq!(
        replay(
            200,
            "application/json",
            vec![b' '; MAX_RESPONSE_BYTES + 1],
            ""
        )
        .await
        .0
        .unwrap_err()
        .machine_code,
        "LEARN_RESPONSE_TOO_LARGE"
    );
}

#[tokio::test]
async fn queued_report_response_is_only_a_post_success_and_keeps_idempotency() {
    for status in [201, 202] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        let result = post_response(
            Url::parse(&format!("http://{address}/reports")).unwrap(),
            "mock-only-token",
            &serde_json::json!({"scope":"course"}),
            "test-request-id",
            1024,
        )
        .await
        .unwrap();
        assert_eq!(result, b"{}");
        let request = server.join().unwrap().to_lowercase();
        assert!(request.starts_with("post /reports "));
        assert!(request.contains("idempotency-key: test-request-id\r\n"));
        let (result, _) = replay(
            status,
            "application/json",
            serde_json::to_vec(&fixture()).unwrap(),
            "",
        )
        .await;
        assert_eq!(result.unwrap_err().machine_code, "LEARN_HTTP_ERROR");
    }
}

async fn write_conflict_fixture(
    content_type: &str,
    body: Vec<u8>,
    declared_size: usize,
) -> (LearnError, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let content_type = content_type.to_owned();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..n]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        if declared_size == usize::MAX {
            stream.write_all(format!("HTTP/1.1 409 Conflict\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", body.len()).as_bytes()).unwrap();
            let _ = stream.write_all(&body);
            let _ = stream.write_all(b"\r\n0\r\n\r\n");
        } else {
            stream.write_all(format!("HTTP/1.1 409 Conflict\r\nContent-Type: {content_type}\r\nContent-Length: {declared_size}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            let _ = stream.write_all(&body);
        }
        String::from_utf8(request).unwrap()
    });
    let e = post_response(
        Url::parse(&format!("http://{address}/records")).unwrap(),
        "mock-only-token",
        &json!({"expectedRevision":1,"text":"original"}),
        "original-request",
        1024,
    )
    .await
    .unwrap_err();
    (e, server.join().unwrap())
}

#[tokio::test]
async fn processing_write_is_not_a_definitive_revision_conflict() {
    let body = br#"{"error":{"code":"LEARN_OPERATION_IN_PROGRESS","message":"secret-canary","retryable":true}}"#.to_vec();
    let (e, request) = write_conflict_fixture("application/json", body.clone(), body.len()).await;
    assert_eq!(e.machine_code, "LEARN_OPERATION_IN_PROGRESS");
    assert!(e.retryable);
    assert!(!serde_json::to_string(&e).unwrap().contains("secret-canary"));
    assert!(request
        .to_lowercase()
        .contains("idempotency-key: original-request\r\n"));
    assert_eq!(
        request.split("\r\n\r\n").nth(1).unwrap(),
        r#"{"expectedRevision":1,"text":"original"}"#
    );
}

#[tokio::test]
async fn only_explicit_non_retryable_revision_rejection_is_a_definite_conflict() {
    for (code, retryable, expected) in [
        ("LEARN_REVISION_CONFLICT", false, "LEARN_PREVIEW_STALE"),
        ("LEARN_PREVIEW_STALE", false, "LEARN_PREVIEW_STALE"),
        ("IDEMPOTENCY_CONFLICT", false, "LEARN_IDEMPOTENCY_CONFLICT"),
        ("LEARN_REVISION_CONFLICT", true, "LEARN_WRITE_UNCONFIRMED"),
        (
            "LEARN_OPERATION_IN_PROGRESS",
            false,
            "LEARN_WRITE_UNCONFIRMED",
        ),
        ("NEW_UNKNOWN_CONFLICT", false, "LEARN_WRITE_UNCONFIRMED"),
    ] {
        let body=serde_json::to_vec(&json!({"error":{"code":code,"retryable":retryable,"message":"secret-canary","details":{"token":"secret-canary"}}})).unwrap();
        let (e, _) =
            write_conflict_fixture("application/json; charset=utf-8", body.clone(), body.len())
                .await;
        assert_eq!(e.machine_code, expected);
        assert!(!serde_json::to_string(&e).unwrap().contains("secret-canary"));
    }
}

#[tokio::test]
async fn opaque_oversized_or_interrupted_conflict_keeps_the_write_unconfirmed() {
    let valid = br#"{"error":{"code":"LEARN_REVISION_CONFLICT","retryable":false}}"#.to_vec();
    for (mime, body, extra_size) in [
        ("text/html", valid.clone(), 0),
        (
            "application/json",
            br#"{"error":{"code":"LEARN_REVISION_CONFLICT"}}"#.to_vec(),
            0,
        ),
        (
            "application/json",
            br#"{"error":{"code":"LEARN_REVISION_CONFLICT","retryable":"false"}}"#.to_vec(),
            0,
        ),
        ("application/json", b"secret-canary not json".to_vec(), 0),
        ("application/json", vec![b'x'; 16385], 0),
        (
            "application/json",
            [valid.clone(), vec![b' '; 16385]].concat(),
            usize::MAX,
        ),
        ("application/json", valid, 20),
    ] {
        let declared = if extra_size == usize::MAX {
            usize::MAX
        } else {
            body.len() + extra_size
        };
        let (e, _) = write_conflict_fixture(mime, body, declared).await;
        assert_eq!(e.machine_code, "LEARN_WRITE_UNCONFIRMED");
        assert!(e.retryable);
        assert!(!serde_json::to_string(&e).unwrap().contains("secret-canary"));
    }
}

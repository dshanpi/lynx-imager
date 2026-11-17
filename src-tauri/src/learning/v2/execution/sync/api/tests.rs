use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

async fn serve(
    request: &Request,
    status: u16,
    body: Vec<u8>,
    extra: &str,
) -> (Result<Delivery, Fault>, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let extra = extra.to_owned();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut received = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            received.extend_from_slice(&buf[..n]);
            if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&received[..end]).to_lowercase();
                let size: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if received.len() >= end + 4 + size {
                    break;
                }
            }
        }
        let header=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",body.len());
        stream.write_all(header.as_bytes()).unwrap();
        let _ = stream.write_all(&body);
        received
    });
    let result = post(&format!("http://{addr}"), "test-token-only", request).await;
    (result, server.join().unwrap())
}
fn session() -> Request {
    Request::new(
        0,
        "stable-key".into(),
        Route::Session,
        json!({"enrollmentId":"enrollment_one","unitId":"unit_one","instanceId":"instance_one"}),
        None,
    )
    .unwrap()
}
#[tokio::test]
async fn raw_journal_body_and_key_are_preserved_across_unknown_response() {
    let mut request = session();
    request.body="{\"unitId\":\"unit_one\", \"enrollmentId\":\"enrollment_one\",\"instanceId\":\"instance_one\"}".into();
    let (result,wire)=serve(&request,409,br#"{"error":{"code":"LEARN_OPERATION_IN_PROGRESS","retryable":true,"message":"secret-canary"}}"#.to_vec(),"").await;
    let error = result.err().unwrap();
    assert_eq!(error.code, "LEARN_OPERATION_IN_PROGRESS");
    assert!(error.retryable);
    let end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(&wire[end..], request.body.as_bytes());
    let text = String::from_utf8(wire).unwrap();
    assert!(text.starts_with("POST /api/v2/lynx/learn/sessions "));
    assert!(text
        .to_lowercase()
        .contains("idempotency-key: stable-key\r\n"));
    assert!(!format!("{error:?}").contains("secret-canary"));
    let (result, wire) = serve(
        &request,
        201,
        br#"{"data":{"sessionId":"session_one","attemptId":"attempt_one","revision":1}}"#.to_vec(),
        "",
    )
    .await;
    assert!(matches!(
        result.unwrap().receipt,
        Receipt::Session { revision: 1, .. }
    ));
    let end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(&wire[end..], request.body.as_bytes());
}
#[tokio::test]
async fn only_fixed_event_route_and_no_redirect_or_oversize_receipt() {
    let request = Request::new(
        1,
        "events-key".into(),
        Route::Events {
            attempt_id: "attempt_one".into(),
        },
        json!({"baseRevision":1,"instanceId":"instance_one","events":[]}),
        None,
    )
    .unwrap();
    let (result, wire) = serve(
        &request,
        302,
        vec![],
        "Location: http://127.0.0.1:1/secret\r\n",
    )
    .await;
    assert_eq!(result.err().unwrap().code, "LEARN_REDIRECT_REJECTED");
    assert!(String::from_utf8(wire)
        .unwrap()
        .starts_with("POST /api/v2/lynx/learn/attempts/attempt_one/events:batch "));
    let (result, _) = serve(&request, 200, vec![b' '; MAX_RESPONSE + 1], "").await;
    assert_eq!(result.err().unwrap().code, "LEARN_WRITE_UNCONFIRMED");
}
#[tokio::test]
async fn expired_completion_is_known_but_unknown_conflict_keeps_original_request() {
    let request = Request::new(
        8,
        "completion-key".into(),
        Route::Completion {
            evidence_id: "evidence_one".into(),
        },
        json!({"uploadId":"upload_one"}),
        None,
    )
    .unwrap();
    let (result, _) = serve(
        &request,
        409,
        br#"{"error":{"code":"LEARN_UPLOAD_EXPIRED","retryable":false}}"#.to_vec(),
        "",
    )
    .await;
    assert_eq!(
        result.unwrap().receipt,
        Receipt::GrantExpired {
            upload_id: "upload_one".into()
        }
    );
    for body in [
        br#"{"error":{"code":"LEARN_UPLOAD_EXPIRED","retryable":true}}"#.to_vec(),
        br#"{"error":{"code":"NEW_ERROR","retryable":false}}"#.to_vec(),
        br#"{"data":{"token":"secret-canary"}}"#.to_vec(),
    ] {
        let (result, _) = serve(&request, 409, body, "").await;
        assert_eq!(result.err().unwrap().code, "LEARN_WRITE_UNCONFIRMED");
    }
}
#[test]
fn grants_validate_exact_private_object_headers_and_keep_url_out_of_receipt() {
    let request = Request::new(
        3,
        "grant-key".into(),
        Route::Grant {
            evidence_id: "evidence_one".into(),
        },
        json!({"contentType":"text/plain","size":3,"sha256":"a".repeat(64)}),
        None,
    )
    .unwrap();
    let fixture = json!({"uploadId":"upload_one","url":"https://bucket.oss-cn-beijing.aliyuncs.com/learn/pending/evidence_one/upload_one?Signature=canary-secret","expiresAt":(chrono::Utc::now()+chrono::Duration::minutes(10)).to_rfc3339(),"headers":{"Content-Type":"text/plain","x-oss-object-acl":"private"}});
    let delivery = parse_data(&request, fixture.clone()).unwrap();
    assert!(!serde_json::to_string(&delivery.receipt)
        .unwrap()
        .contains("canary-secret"));
    for change in [
        "url", "header", "mime", "path", "scheme", "user", "fragment",
    ] {
        let mut bad = fixture.clone();
        match change {
            "url"=>bad["url"]=json!("https://127.0.0.1/learn/pending/evidence_one/upload_one?Signature=x"),
            "header"=>bad["headers"]["Authorization"]=json!("Bearer secret"),
            "mime"=>bad["headers"]["Content-Type"]=json!("text/html"),
            "path"=>bad["url"]=json!("https://bucket.example.org/learn/pending/evidence_other/upload_one?Signature=x"),
            "scheme"=>bad["url"]=json!("http://bucket.example.org/learn/pending/evidence_one/upload_one?Signature=x"),
            "user"=>bad["url"]=json!("https://user:pass@bucket.example.org/learn/pending/evidence_one/upload_one?Signature=x"),
            _=>bad["url"]=json!("https://bucket.example.org/learn/pending/evidence_one/upload_one?Signature=x#secret"),
        }
        assert!(parse_data(&request, bad).is_err(), "accepted {change}");
    }
}

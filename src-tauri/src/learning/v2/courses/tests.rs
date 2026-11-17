use super::*;
use crate::learning::v2::manifest::tests::{fixture, verified};
use serde_json::json;

fn content() -> LearnUnitContent {
    let text = "# Hello Linux\n\n这是测试教材。\n";
    LearnUnitContent {
        enrollment_id: "enr_test".into(),
        release_id: "release_demo_1".into(),
        unit_id: "unit_hello".into(),
        documents: vec![DocumentSnapshot {
            resource_id: "doc_intro".into(),
            sha256: hex::encode(Sha256::digest(text.as_bytes())),
            markdown: text.into(),
        }],
    }
}
#[test]
fn documents_are_exact_signed_utf8_bytes_and_scoped_to_unit() {
    let course = verified();
    validate_documents(&content(), &course, "unit_hello").unwrap();
    let mut bad = content();
    bad.documents[0].markdown.push('!');
    assert!(validate_documents(&bad, &course, "unit_hello").is_err());
    let mut bad = content();
    bad.enrollment_id = "another-user".into();
    assert!(validate_documents(&bad, &course, "unit_hello").is_err());
    let mut bad = content();
    bad.unit_id = "another-unit".into();
    assert!(validate_documents(&bad, &course, "unit_hello").is_err());
    let mut bad = content();
    bad.documents.clear();
    assert!(validate_documents(&bad, &course, "unit_hello").is_err());
    assert!(validate_documents(&content(), &course, "absent").is_err());
}
#[test]
fn directory_and_path_validation_do_not_allow_duplicate_enrollments_or_path_injection() {
    let (_, e, _) = fixture();
    let valid = json!({"data":{"enrolled":[e]}});
    assert_eq!(
        parse_home(&serde_json::to_vec(&valid).unwrap())
            .unwrap()
            .len(),
        1
    );
    assert!(parse_home(&serde_json::to_vec(&json!({"data":{"enrolled":[e,e]}})).unwrap()).is_err());
    assert!(parse_home(b"{\"data\":{}}").is_err());
    assert!(url("https://www.100ask.net", &["enrollments", "../../private"]).is_err());
    assert_eq!(
        url("https://www.100ask.net", &["enrollments", "e1", "manifest"])
            .unwrap()
            .path(),
        "/api/v2/lynx/learn/enrollments/e1/manifest"
    );
}
#[tokio::test]
async fn real_http_replay_binds_directory_manifest_and_documents() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };
    let (r, e, keys) = fixture();
    let bodies = [
        json!({"data":{"enrolled":[e]}}),
        json!({"data":{"contentEnvelope":r.content_envelope,"policyEnvelope":r.policy_envelope,"entitlement":{"state":"active"}}}),
        json!({"data":content()}),
    ];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let mut requests = vec![];
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0; 1024];
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let body = body.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        requests
    });
    let base = format!("http://{address}");
    let verified = course(
        &base,
        "mock-token",
        &e.enrollment_id,
        &e.release_id,
        &e.manifest_sha256,
        &keys,
    )
    .await
    .unwrap();
    let c: LearnUnitContent = read(
        &base,
        "mock-token",
        &["enrollments", &e.enrollment_id, "units", "unit_hello"],
    )
    .await
    .unwrap();
    validate_documents(&c, &verified, "unit_hello").unwrap();
    let requests = task.join().unwrap();
    assert!(requests[0].starts_with("GET /api/v2/lynx/learn/home "));
    assert!(requests.iter().all(|r| r
        .to_lowercase()
        .contains("authorization: bearer mock-token")));
    assert!(requests[1].starts_with("GET /api/v2/lynx/learn/enrollments/enr_test/manifest "));
    assert!(
        requests[2].starts_with("GET /api/v2/lynx/learn/enrollments/enr_test/units/unit_hello ")
    );
}

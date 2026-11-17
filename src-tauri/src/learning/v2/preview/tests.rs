use super::*;
use crate::learning::v2::manifest::tests::{fixture, sign};
use serde_json::{json, Value};
pub(super) fn preview_fixture() -> (PreviewResponse, TrustedKeys) {
    let (r, _, keys) = fixture();
    let mut manifest: Value = serde_json::from_slice(
        &base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &r.content_envelope.payload,
        )
        .unwrap(),
    )
    .unwrap();
    manifest["releaseId"] = json!("preview_test");
    manifest["version"] = json!("0.0.0-preview.3");
    let content = sign(&manifest);
    let mut policy: Value = serde_json::from_slice(
        &base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &r.policy_envelope.payload,
        )
        .unwrap(),
    )
    .unwrap();
    policy["releaseId"] = json!("preview_test");
    policy["manifestSha256"] = json!(content.sha256);
    let policy = sign(&policy);
    let binding = sign(
        &json!({"format":"100ask.learn-preview-binding","schemaVersion":1,"previewId":"preview_test","userId":7,"draftId":"draft_test","draftRevision":3,"contentDigest":"a".repeat(64),"courseId":manifest["courseId"],"releaseId":"preview_test","manifestSha256":content.sha256,"policySha256":policy.sha256,"allowedUnitIds":["unit_hello"],"expiresAt":"2099-01-01T00:00:00Z","scope":"draft_readonly"}),
    );
    (
        PreviewResponse {
            content_envelope: content,
            policy_envelope: policy,
            binding_envelope: binding,
        },
        keys,
    )
}
#[test]
fn signed_preview_binds_account_invitation_all_units_and_readonly_scope() {
    let (r, k) = preview_fixture();
    let p = verify_preview(r, "preview_test", 7, &k).unwrap();
    assert!(!p.course.execution_enabled);
    assert_eq!(p.binding.scope, "draft_readonly");
    let (r, k) = preview_fixture();
    assert!(verify_preview(r, "other", 7, &k).is_err());
    let (r, k) = preview_fixture();
    assert!(verify_preview(r, "preview_test", 8, &k).is_err());
    for field in [
        "expiresAt",
        "allowedUnitIds",
        "policySha256",
        "scope",
        "releaseId",
    ] {
        let (mut r, k) = preview_fixture();
        let mut b: Value = manifest::verify(&r.binding_envelope, &k, 65536).unwrap();
        b[field] = match field {
            "expiresAt" => json!("2020-01-01T00:00:00Z"),
            "allowedUnitIds" => json!([]),
            "policySha256" => json!("b".repeat(64)),
            _ => json!("other"),
        };
        r.binding_envelope = sign(&b);
        assert!(verify_preview(r, "preview_test", 7, &k).is_err(), "{field}");
    }
}
#[test]
fn signed_read_receipt_cannot_claim_execution_or_different_document_evidence() {
    let (r, k) = preview_fixture();
    let p = verify_preview(r, "preview_test", 7, &k).unwrap();
    let docs = vec![DocumentCheck {
        unit_id: "unit_hello".into(),
        resource_id: "doc_intro".into(),
        sha256: "a".repeat(64),
    }];
    let value = json!({"format":"100ask.learn-preview-read-receipt","schemaVersion":1,"receiptId":"receipt_test","previewId":"preview_test","userId":7,"draftRevision":3,"contentDigest":p.binding.content_digest,"manifestSha256":p.course.manifest_sha256,"scope":"read_integration","status":"server_resources_verified","clientObservationTrust":"client_reported","publicationEligible":false,"clientVersion":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"verifiedAt":chrono::Utc::now().to_rfc3339(),"documents":docs});
    let verify = |v: Value| {
        verify_receipt(
            ReceiptResponse {
                receipt_id: "receipt_test".into(),
                receipt_envelope: sign(&v),
            },
            &p,
            &docs,
            &k,
        )
    };
    assert!(verify(value.clone()).is_ok());
    for field in [
        "publicationEligible",
        "userId",
        "documents",
        "scope",
        "contentDigest",
    ] {
        let mut v = value.clone();
        v[field] = match field {
            "publicationEligible" => json!(true),
            "userId" => json!(8),
            "documents" => json!([]),
            _ => json!("wrong"),
        };
        assert!(verify(v).is_err(), "{field}");
    }
}

#[tokio::test]
#[ignore = "requires opt-in Go website bridge on SSH-forwarded loopback"]
async fn live_go_website_signed_preview_documents_and_receipt() {
    let raw =
        std::fs::read(std::env::var("LYNX_PREVIEW_BRIDGE_DESCRIPTOR").expect("bridge descriptor"))
            .unwrap();
    let data: Value = serde_json::from_slice(&raw).unwrap();
    let base = std::env::var("LYNX_PREVIEW_BRIDGE_URL").unwrap();
    let id = data["previewId"].as_str().unwrap();
    let keys = manifest::parse_trusted_keys(&data["publicKeys"].to_string()).unwrap();
    let p = preview(&base, "valid-test-token", id, 7, &keys)
        .await
        .unwrap();
    assert_eq!(p.course.manifest.track, "kvm");
    assert_eq!(p.binding.allowed_unit_ids.len(), 3);
    assert!(preview(&base, "old-test-token", id, 7, &keys)
        .await
        .is_err());
    assert!(preview(&base, "valid-test-token", id, 8, &keys)
        .await
        .is_err());
    let doc = unit(&base, "valid-test-token", &p, "unit_0").await.unwrap();
    assert!(doc.documents[0].markdown.contains("Web KVM"));
    let receipt = read_check(
        &base,
        "valid-test-token",
        &p,
        &keys,
        "desktop-bridge-receipt",
    )
    .await
    .unwrap();
    assert_eq!(receipt.documents.len(), 3);
    assert!(!receipt.publication_eligible);
    let again = read_check(
        &base,
        "valid-test-token",
        &p,
        &keys,
        "desktop-bridge-receipt",
    )
    .await
    .unwrap();
    assert_eq!(receipt.receipt_id, again.receipt_id);
    let saved: ReceiptResponse = courses::read(
        &base,
        "valid-test-token",
        &["previews", id, "read-receipts", &receipt.receipt_id],
    )
    .await
    .unwrap();
    assert_eq!(
        verify_receipt(saved, &p, &receipt.documents, &keys)
            .unwrap()
            .receipt_id,
        receipt.receipt_id
    );
}

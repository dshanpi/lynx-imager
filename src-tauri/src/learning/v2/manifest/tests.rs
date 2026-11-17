use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};

// Deterministic public test vector only. Never included in the production trust registry.
pub(crate) fn sign(value: &Value) -> SignedEnvelope {
    let bytes = serde_json::to_vec(value).unwrap();
    let digest = Sha256::digest(&bytes);
    SignedEnvelope {
        format: "100ask.learn-envelope".into(),
        schema_version: 1,
        key_id: "test-only-20261008".into(),
        algorithm: "Ed25519-SHA256".into(),
        payload_encoding: "base64".into(),
        payload: STANDARD.encode(&bytes),
        sha256: hex::encode(digest),
        signature: STANDARD.encode(SigningKey::from_bytes(&[42; 32]).sign(&digest).to_bytes()),
    }
}
pub(crate) fn fixture() -> (ManifestResponse, EnrollmentSummary, TrustedKeys) {
    let mut data: Value = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/course.json"
    ))
    .unwrap();
    let doc = "# Hello Linux\n\n这是测试教材。\n";
    data["resources"][0]["sha256"] = json!(hex::encode(Sha256::digest(doc.as_bytes())));
    data["resources"][0]["size"] = json!(doc.len());
    let content = sign(&data);
    let policy = sign(
        &json!({"format":"100ask.learn-policy","schemaVersion":1,"releaseId":data["releaseId"],"manifestSha256":content.sha256,"agentPolicy":{"enabled":false,"allowedToolIds":[],"hintLevels":["explain","hint"],"contextResourceRefs":["doc_intro"],"maxContextBytes":65536,"allowImageAnalysis":false}}),
    );
    let enrollment = EnrollmentSummary {
        course_slug: None,
        enrollment_id: "enr_test".into(),
        course_id: data["courseId"].as_str().unwrap().into(),
        release_id: data["releaseId"].as_str().unwrap().into(),
        manifest_sha256: content.sha256.clone(),
        version: "1.0.0".into(),
        title: "课程".into(),
        summary: "".into(),
        track: "linux_app".into(),
        current_unit_id: None,
    };
    let keys = HashMap::from([(
        "test-only-20261008".into(),
        SigningKey::from_bytes(&[42; 32]).verifying_key(),
    )]);
    (
        ManifestResponse {
            content_envelope: content,
            policy_envelope: policy,
            entitlement: Entitlement {
                state: "active".into(),
            },
        },
        enrollment,
        keys,
    )
}
pub(crate) fn verified() -> VerifiedCourse {
    let (r, e, k) = fixture();
    verify_course(r, &e, &k).unwrap()
}

#[test]
fn exact_binary_digest_signature_and_separate_policy_are_required() {
    let (r, e, k) = fixture();
    let verified = verify_course(r, &e, &k).unwrap();
    assert_eq!(verified.manifest.chapters.len(), 1);
    assert!(!verified.execution_enabled);
    assert!(verified
        .missing_capabilities
        .contains(&"learn.ssh-workspace.v1".into()));
    let (mut r, e, k) = fixture();
    r.content_envelope.signature = STANDARD.encode(
        SigningKey::from_bytes(&[42; 32])
            .sign(r.content_envelope.sha256.as_bytes())
            .to_bytes(),
    );
    assert_eq!(
        verify_course(r, &e, &k).unwrap_err().machine_code,
        "LEARN_SIGNATURE_INVALID"
    );
    let (mut r, e, k) = fixture();
    r.policy_envelope = sign(
        &json!({"format":"100ask.learn-policy","schemaVersion":1,"releaseId":"other","manifestSha256":e.manifest_sha256,"agentPolicy":{"enabled":false,"allowedToolIds":[],"hintLevels":[],"contextResourceRefs":[],"maxContextBytes":0,"allowImageAnalysis":false}}),
    );
    assert_eq!(
        verify_course(r, &e, &k).unwrap_err().machine_code,
        "LEARN_RELEASE_MISMATCH"
    );
}
#[test]
fn untrusted_signer_payload_tampering_and_revocation_fail_closed() {
    let (mut r, e, k) = fixture();
    r.content_envelope.key_id = "forged-official".into();
    assert_eq!(
        verify_course(r, &e, &k).unwrap_err().machine_code,
        "LEARN_SIGNER_UNTRUSTED"
    );
    let (mut r, e, k) = fixture();
    r.content_envelope.payload = STANDARD.encode(b"{\"modified\":true}");
    assert_eq!(
        verify_course(r, &e, &k).unwrap_err().machine_code,
        "LEARN_DIGEST_MISMATCH"
    );
    let (mut r, e, k) = fixture();
    r.entitlement.state = "revoked".into();
    assert_eq!(
        verify_course(r, &e, &k).unwrap_err().machine_code,
        "LEARN_ENTITLEMENT_REQUIRED"
    );
    let (r, mut e, k) = fixture();
    e.release_id = "another-release".into();
    assert!(verify_course(r, &e, &k).is_err());
}
#[test]
fn namespaced_schema_theory_and_arbitrary_chapter_counts() {
    let theory: CourseManifest = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/theory.json"
    ))
    .unwrap();
    validate_manifest(&theory).unwrap();
    assert_eq!(theory.environments[0].target, "none");
    assert!(theory.actions.is_empty());
    let mut m = verified().manifest;
    m.schema_version = 3;
    assert!(validate_manifest(&m).is_err());
    m.schema_version = 1;
    m.chapters.clear();
    assert!(validate_manifest(&m).is_err());
}
#[test]
fn missing_duplicate_cross_environment_refs_and_unpinned_source_rejected() {
    let original = verified().manifest;
    let mut m = original.clone();
    m.resources[1].resource_id = m.resources[0].resource_id.clone();
    assert!(validate_manifest(&m).is_err());
    let mut m = original.clone();
    m.chapters[0].units[0].activities[0].resource_refs = vec!["missing".into()];
    assert!(validate_manifest(&m).is_err());
    let mut m = original.clone();
    m.chapters[0].units[0].activities[1].rule_refs.clear();
    assert!(validate_manifest(&m).is_err());
    let mut m = original.clone();
    m.actions[0].target = "board".into();
    assert!(validate_manifest(&m).is_err());
    let mut m = original.clone();
    m.resources[1].commit = None;
    assert!(validate_manifest(&m).is_err());
    let mut m = original;
    m.resources[0].source_url = "https://user:secret@example.org/private".into();
    assert!(validate_manifest(&m).is_err());
}
#[test]
fn official_trust_registry_rejects_empty_duplicate_and_weak_keys() {
    assert!(parse_trusted_keys("[]").is_err());
    assert!(parse_trusted_keys("{}").is_err());
    let entry = json!({"keyId":"official-1","publicKey":STANDARD.encode(SigningKey::from_bytes(&[42;32]).verifying_key().as_bytes())});
    assert_eq!(
        parse_trusted_keys(&json!([entry]).to_string())
            .unwrap()
            .len(),
        1
    );
    assert!(parse_trusted_keys(&json!([entry, entry]).to_string()).is_err());
    assert!(parse_trusted_keys(
        &json!([{"keyId":"weak","publicKey":STANDARD.encode([0;32])}]).to_string()
    )
    .is_err());
}
#[test]
fn envelope_limits_and_unknown_fields_are_rejected() {
    let (mut r, e, k) = fixture();
    r.content_envelope.payload = "A".repeat(MAX_MANIFEST_BYTES * 2);
    assert!(verify_course(r, &e, &k).is_err());
    let mut value: Value = serde_json::to_value(verified().manifest).unwrap();
    value["publicKey"] = json!("untrusted");
    assert!(serde_json::from_value::<CourseManifest>(value).is_err());
}

#[test]
fn node_generated_digest_signatures_match_rust_verification() {
    let keys = parse_trusted_keys(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/test-trust-registry.json"
    ))
    .unwrap();
    let home: Value = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/home.response.json"
    ))
    .unwrap();
    let enrollment: EnrollmentSummary =
        serde_json::from_value(home["data"]["enrolled"][0].clone()).unwrap();
    let envelope: Value = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/manifest.response.json"
    ))
    .unwrap();
    let response: ManifestResponse = serde_json::from_value(envelope["data"].clone()).unwrap();
    let verified = verify_course(response, &enrollment, &keys).unwrap();
    assert_eq!(verified.manifest_sha256, enrollment.manifest_sha256);
    assert!(!verified.execution_enabled);
}

#[test]
fn optional_resource_fields_can_be_absent_but_not_explicit_null() {
    let video = json!({"resourceId":"video1","kind":"video","title":"Video","sourceUrl":"https://example.org/lesson"});
    assert!(serde_json::from_value::<CourseResource>(video.clone()).is_ok());
    let mut invalid = video;
    invalid["sha256"] = Value::Null;
    assert!(serde_json::from_value::<CourseResource>(invalid).is_err());
}

#[test]
fn manifest_v2_distinguishes_action_instances_from_capabilities() {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/learn-v2/fixtures/course.json"
    ))
    .unwrap();
    value["schemaVersion"] = json!(2);
    value["requiredClientCapabilities"][0] = json!("learn.course-manifest.v2");
    value["actions"][0]["capabilityId"] = json!("compile_hello");
    value["actions"][1]["capabilityId"] = json!("run_hello");
    value["actions"][0]["actionId"] = json!("build_first");
    value["chapters"][0]["units"][0]["activities"][1]["actionRefs"][0] = json!("build_first");
    let mut second = value["actions"][0].clone();
    second["actionId"] = json!("build_second");
    value["actions"].as_array_mut().unwrap().push(second);
    let parsed: CourseManifest = serde_json::from_value(value.clone()).unwrap();
    validate_manifest(&parsed).unwrap();
    value["actions"][0]
        .as_object_mut()
        .unwrap()
        .remove("capabilityId");
    assert!(validate_manifest(&serde_json::from_value(value.clone()).unwrap()).is_err());
    value["actions"][0]["capabilityId"] = json!(null);
    assert!(serde_json::from_value::<CourseManifest>(value.clone()).is_err());
    value["actions"][0]["capabilityId"] = json!("compile_hello");
    value["schemaVersion"] = json!(1);
    assert!(validate_manifest(&serde_json::from_value(value).unwrap()).is_err());
}

#[test]
fn default_client_contains_confirmed_official_course_root() {
    // This regression is run without a compile-time override. Controlled builds
    // may deliberately supply a different registry for key rotation.
    if option_env!("LYNX_LEARN_TRUSTED_KEYS_JSON").is_none_or(|s| s.trim().is_empty()) {
        let keys = official_keys().expect("default client must embed its official course root");
        assert!(keys.contains_key("learn-e517f9c17a132ea0"));
        assert!(!keys.contains_key("test-only-20261008"));
        let (response, _, _) = fixture();
        let mut envelope = response.content_envelope;
        assert_eq!(
            verify::<CourseManifest>(&envelope, &keys, MAX_MANIFEST_BYTES)
                .unwrap_err()
                .machine_code,
            "LEARN_SIGNER_UNTRUSTED"
        );
        envelope.key_id = "learn-e517f9c17a132ea0".into();
        assert_eq!(
            verify::<CourseManifest>(&envelope, &keys, MAX_MANIFEST_BYTES)
                .unwrap_err()
                .machine_code,
            "LEARN_SIGNATURE_INVALID"
        );
    }
}

#[test]
fn signed_board_observation_course_negotiates_implemented_capability() {
    let (mut response, mut enrollment, keys) = fixture();
    let mut course = verified().manifest;
    course.schema_version = 2;
    course.required_client_capabilities = vec![
        "learn.course-manifest.v2".into(),
        "learn.board-observation.v1".into(),
    ];
    for action in &mut course.actions {
        action.capability_id = Some("learn.board.observe".into());
        action.catalog_version = "1.0.0".into();
        action.target = "board".into();
        action.risk = "read_only".into();
        action.parameters.clear();
        action.evidence_outputs = vec!["board_observation".into()];
    }
    for profile in &mut course.environments {
        if profile.target == "ubuntu" {
            profile.target = "board".into();
        }
    }
    let mut value = serde_json::to_value(course).unwrap();
    for resource in value["resources"].as_array_mut().unwrap() {
        resource
            .as_object_mut()
            .unwrap()
            .retain(|_, value| !value.is_null());
    }
    response.content_envelope = sign(&value);
    enrollment.manifest_sha256 = response.content_envelope.sha256.clone();
    let mut policy: Value =
        serde_json::from_slice(&STANDARD.decode(&response.policy_envelope.payload).unwrap())
            .unwrap();
    policy["manifestSha256"] = json!(enrollment.manifest_sha256);
    response.policy_envelope = sign(&policy);
    let course = verify_course(response, &enrollment, &keys).unwrap();
    assert!(course.missing_capabilities.is_empty());
    assert!(course.execution_enabled);
}

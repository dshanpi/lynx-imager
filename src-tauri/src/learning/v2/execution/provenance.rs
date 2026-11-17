//! Execution-time facts are observations, never a server/device attestation.
use super::*;

#[derive(Clone, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentFacts {
    pub os: String,
    pub architecture: String,
    pub python_version: String,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionContext {
    pub collector_version: String,
    pub source_commit: String,
    pub source_fingerprint: String,
    pub definition_sha256: String,
    pub facts: EnvironmentFacts,
    pub fingerprint: String,
    pub observed_at: String,
}

pub(super) fn context(saved: &Stored) -> Result<Option<ExecutionContext>, String> {
    let Some(raw) = saved.run.result.get("executionContext") else {
        return Ok(None);
    };
    let invalid = "LEARN_EXECUTION_CONTEXT_INVALID";
    let c: ExecutionContext = serde_json::from_value(raw.clone()).map_err(|_| invalid)?;
    let task = &saved.run.result["task"];
    let observed = chrono::DateTime::parse_from_rfc3339(&c.observed_at).map_err(|_| invalid)?;
    let started = chrono::DateTime::parse_from_rfc3339(
        saved.run.result["startedAt"].as_str().ok_or(invalid)?,
    )
    .map_err(|_| invalid)?;
    if c.collector_version != "lynx-project-context/1"
        || ![40, 64].contains(&c.source_commit.len())
        || !c
            .source_commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !manifest::valid_digest(&c.source_fingerprint)
        || !manifest::valid_digest(&c.definition_sha256)
        || !manifest::valid_digest(&c.fingerprint)
        || c.observed_at.len() > 64
        || observed > started
        || [&c.facts.os, &c.facts.architecture, &c.facts.python_version]
            .iter()
            .any(|s| s.len() > 512 || s.contains('\0'))
        || task["commit"].as_str() != Some(&c.source_commit)
        || task["learningSourceFingerprint"].as_str() != Some(&c.source_fingerprint)
        || task["definitionSha256"].as_str() != Some(&c.definition_sha256)
        || serde_json::to_value(&c.facts).map_err(|_| invalid)? != task["environmentFacts"]
    {
        return Err(invalid.into());
    }
    // serde_json::Value maps and the Python collector both sort keys; UTF-8
    // strings are encoded directly, including non-ASCII OS descriptions.
    let mut canonical = serde_json::to_value(&c).map_err(|_| invalid)?;
    canonical
        .as_object_mut()
        .ok_or(invalid)?
        .remove("fingerprint");
    canonical
        .as_object_mut()
        .ok_or(invalid)?
        .remove("observedAt");
    if hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).map_err(|_| invalid)?,
    )) != c.fingerprint
    {
        return Err(invalid.into());
    }
    Ok(Some(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn saved() -> Stored {
        let mut s = super::super::tests::saved_fixture();
        let mut c = json!({"collectorVersion":"lynx-project-context/1","sourceCommit":"a".repeat(40),"sourceFingerprint":"b".repeat(64),"definitionSha256":"c".repeat(64),"facts":{"os":"Ubuntu 测试","architecture":"aarch64","pythonVersion":"3.12.0"}});
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&c).unwrap()));
        c["fingerprint"] = json!(digest);
        c["observedAt"] = json!("2026-10-18T00:00:00Z");
        s.run.result = json!({"startedAt":"2026-10-18T00:00:01Z","executionContext":c,"task":{"commit":c["sourceCommit"],"learningSourceFingerprint":c["sourceFingerprint"],"definitionSha256":c["definitionSha256"],"environmentFacts":c["facts"]}});
        s
    }
    #[test]
    fn validates_actual_context_but_never_invents_legacy_observations() {
        let s = saved();
        assert_eq!(context(&s).unwrap().unwrap().facts.os, "Ubuntu 测试");
        let mut legacy = saved();
        legacy
            .run
            .result
            .as_object_mut()
            .unwrap()
            .remove("executionContext");
        assert!(context(&legacy).unwrap().is_none());
    }
    #[test]
    fn rejects_changed_facts_wrong_source_invalid_time_and_extra_fields() {
        for (field, value) in [
            ("sourceCommit", json!("d".repeat(40))),
            ("fingerprint", json!("d".repeat(64))),
            ("sourceFingerprint", json!("d".repeat(64))),
            ("definitionSha256", json!("d".repeat(64))),
            ("collectorVersion", json!("server-verified")),
            ("observedAt", json!("2026-10-18T00:00:02Z")),
            ("hostname", json!("private")),
        ] {
            let mut s = saved();
            s.run.result["executionContext"][field] = value;
            assert!(context(&s).is_err(), "accepted {field}");
        }
        let mut s = saved();
        s.run.result["task"]["environmentFacts"]["architecture"] = json!("other");
        assert!(context(&s).is_err());
        let mut s = saved();
        s.run.result["executionContext"] = Value::Null;
        assert!(context(&s).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn python_and_rust_use_the_same_unicode_context_digest() {
        let mut s = saved();
        let script="import importlib.util,json,sys;spec=importlib.util.spec_from_file_location('runner',sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);print(json.dumps(m.execution_context(json.load(sys.stdin))))";
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                script,
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/learning/v2/project_runner.py"
                ),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::to_string(&s.run.result["task"])
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        s.run.result["executionContext"] = serde_json::from_slice(&output.stdout).unwrap();
        s.run.result["startedAt"] = json!(chrono::Utc::now().to_rfc3339());
        assert!(context(&s).unwrap().is_some());
    }
}

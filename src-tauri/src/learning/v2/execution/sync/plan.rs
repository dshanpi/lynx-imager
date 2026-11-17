//! Build one immutable unit batch from already validated execution-time previews.
use super::super::*;
use super::journal::{EvidenceRef, FrozenBatch, RunRef};

fn digest(value: &Value) -> Result<String, String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(|_| "LEARN_SYNC_PLAN_INVALID")?,
    )))
}
/// Inputs must first pass evidence::restore inside the same transaction as enqueue.
pub(super) fn compile(
    course: &manifest::VerifiedCourse,
    batch_id: &str,
    instance_id: &str,
    entries: &[(Stored, evidence::EvidencePreview)],
) -> Result<FrozenBatch, String> {
    let invalid = "LEARN_SYNC_PLAN_INVALID";
    if !manifest::valid_id(batch_id)
        || !manifest::valid_id(instance_id)
        || entries.is_empty()
        || entries.len() > 8
        || entries.iter().any(|(_, p)| p.items.is_empty())
        || !course.execution_enabled
    {
        return Err(invalid.into());
    }
    let mut ordered: Vec<_> = entries.iter().collect();
    ordered.sort_by(|a, b| {
        a.1.items[0]
            .started_at
            .cmp(&b.1.items[0].started_at)
            .then(a.0.run.run_id.cmp(&b.0.run.run_id))
    });
    let (first, preview) = ordered[0];
    let context = preview
        .execution_context
        .as_ref()
        .ok_or("LEARN_SYNC_CONTEXT_MISSING")?;
    let env = &preview.request.environment;
    let fingerprint = digest(
        &json!({"bindingDigest":preview.binding_digest,"contextFingerprint":context.fingerprint}),
    )?;
    let snapshot_id = format!(
        "env_{}",
        &digest(&json!({"batchId":batch_id,"fingerprint":fingerprint}))?[..32]
    );
    let mut actions = std::collections::HashSet::new();
    let mut runs = Vec::new();
    let mut evidence_refs = Vec::new();
    let mut events = Vec::new();
    let event = |sequence: usize,
                 kind: &str,
                 time: &str,
                 payload: Value|
     -> Result<Value, String> {
        let id = format!(
            "event_{}",
            &digest(&json!({"batchId":batch_id,"sequence":sequence}))?[..32]
        );
        Ok(
            json!({"eventId":id,"sequence":sequence,"type":kind,"occurredAt":time,"payload":payload}),
        )
    };
    let observed_at = ordered
        .iter()
        .filter_map(|(_, p)| p.execution_context.as_ref().map(|c| c.observed_at.as_str()))
        .min()
        .ok_or(invalid)?;
    events.push(event(1,"environment.observed",observed_at,json!({"snapshotId":snapshot_id,"fingerprint":fingerprint,"collectorVersion":"lynx-project-context/1","observedAt":observed_at,"components":[
        {"name":"os","expected":"","observed":context.facts.os,"status":"unknown"},
        {"name":"architecture","expected":"","observed":context.facts.architecture,"status":"unknown"},
        {"name":"python","expected":"","observed":context.facts.python_version,"status":"unknown"},
        {"name":"source.commit","expected":context.source_commit,"observed":context.source_commit,"status":"matched"},
        {"name":"source.sha256","expected":"","observed":context.source_fingerprint,"status":"unknown"}
    ]}))?);
    for (saved, p) in ordered {
        let c = p
            .execution_context
            .as_ref()
            .ok_or("LEARN_SYNC_CONTEXT_MISSING")?;
        let (commit, task_id) = task(course, &p.request)?;
        if p.items.is_empty()
            || p.run_id != saved.run.run_id
            || p.binding_digest != saved.binding_digest
            || serde_json::to_value(&p.request).map_err(|_| invalid)?
                != serde_json::to_value(&saved.run.request).map_err(|_| invalid)?
            || p.request.environment.enrollment_id != course.enrollment_id
            || p.request.environment.release_id != course.manifest.release_id
            || p.request.environment.manifest_sha256 != course.manifest_sha256
            || serde_json::to_value(&p.request.environment).map_err(|_| invalid)?
                != serde_json::to_value(env).map_err(|_| invalid)?
            || p.request.unit_id != preview.request.unit_id
            || saved.run.project_id != first.run.project_id
            || p.binding_digest != preview.binding_digest
            || c.fingerprint != context.fingerprint
            || c.source_commit != context.source_commit
            || c.source_fingerprint != context.source_fingerprint
            || c.facts != context.facts
            || c.source_commit != commit
            || saved.run.result["task"]["taskId"].as_str() != Some(&task_id)
            || !actions.insert(&p.request.action_id)
        {
            return Err("LEARN_SYNC_SELECTION_INCOMPATIBLE".into());
        }
        let action = course
            .manifest
            .actions
            .iter()
            .find(|a| a.action_id == p.request.action_id)
            .ok_or(invalid)?;
        let state = match saved.run.state.as_str() {
            "succeeded" => "succeeded",
            "cancelled" => "cancelled",
            "failed" | "timed_out" | "output_limit" => "failed",
            _ => return Err(invalid.into()),
        };
        let start = saved.run.result["startedAt"].as_str().ok_or(invalid)?;
        let end = saved.run.result["finishedAt"].as_str().ok_or(invalid)?;
        let exit = saved.run.result["exitCode"].as_i64().ok_or(invalid)?;
        events.push(event(events.len()+1,"action.started",start,json!({"runId":p.run_id,"actionId":p.request.action_id,"actionVersion":action.catalog_version,"parameterDigest":digest(&json!(action.parameters))?,"environmentSnapshotId":snapshot_id,"startedAt":start}))?);
        events.push(event(events.len()+1,"action.finished",end,json!({"runId":p.run_id,"status":state,"exitCode":exit,"finishedAt":end,"artifactDigests":[]}))?);
        runs.push(RunRef {
            run_id: p.run_id.clone(),
            snapshot_sha256: p.snapshot_sha256.clone(),
        });
        let before = evidence_refs.len();
        for item in &p.items {
            if action.evidence_outputs.contains(&item.kind) {
                evidence_refs.push(EvidenceRef {
                    run_id: p.run_id.clone(),
                    kind: item.kind.clone(),
                    sha256: item.sha256.clone(),
                    size: item.size,
                    content_type: item.content_type.clone(),
                    observed_at: item.finished_at.clone(),
                });
            }
        }
        if evidence_refs.len() == before {
            return Err("LEARN_SYNC_NO_DECLARED_EVIDENCE".into());
        }
    }
    let batch = FrozenBatch {
        batch_id: batch_id.into(),
        enrollment_id: env.enrollment_id.clone(),
        release_id: env.release_id.clone(),
        manifest_sha256: env.manifest_sha256.clone(),
        unit_id: preview.request.unit_id.clone(),
        profile_id: env.profile_id.clone(),
        binding_digest: preview.binding_digest.clone(),
        context_fingerprint: fingerprint,
        instance_id: instance_id.into(),
        runs,
        evidence: evidence_refs,
        event_body: json!({"baseRevision":1,"instanceId":instance_id,"events":events}).to_string(),
    };
    super::journal::validate_batch(&batch)?;
    Ok(batch)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in super::super) fn input() -> (manifest::VerifiedCourse, Stored, evidence::EvidencePreview)
    {
        let (mut course, _) = super::super::super::tests::fixture();
        course.execution_enabled = true;
        course.manifest.actions[0].evidence_outputs =
            vec!["stdout".into(), "build_log".into(), "exit_status".into()];
        let mut s = super::super::super::tests::saved_fixture();
        s.run.state = "succeeded".into();
        s.run.result = json!({"exitCode":0,"startedAt":"2026-10-18T00:00:01Z","finishedAt":"2026-10-18T00:00:02Z","task":{"taskId":"build"}});
        let context = super::super::super::provenance::ExecutionContext {
            collector_version: "lynx-project-context/1".into(),
            source_commit: "c".repeat(40),
            source_fingerprint: "d".repeat(64),
            definition_sha256: "e".repeat(64),
            facts: super::super::super::provenance::EnvironmentFacts {
                os: "Ubuntu".into(),
                architecture: "aarch64".into(),
                python_version: "3.12.0".into(),
            },
            fingerprint: "f".repeat(64),
            observed_at: "2026-10-18T00:00:00Z".into(),
        };
        let item = |kind: &str, text: &str| evidence::EvidenceItem {
            kind: kind.into(),
            state: "succeeded".into(),
            started_at: "2026-10-18T00:00:01Z".into(),
            finished_at: "2026-10-18T00:00:02Z".into(),
            content_type: if kind == "exit_status" {
                "application/json"
            } else {
                "text/plain"
            }
            .into(),
            sha256: hex::encode(Sha256::digest(text.as_bytes())),
            size: text.len(),
            raw_sha256: hex::encode(Sha256::digest(text.as_bytes())),
            raw_bytes: text.len(),
            redacted: false,
            output_truncated: false,
            collector_version: "lynx-project-evidence/1".into(),
            text: text.into(),
        };
        let p = evidence::EvidencePreview {
            snapshot_sha256: "a".repeat(64),
            run_id: s.run.run_id.clone(),
            request: s.run.request.clone(),
            binding_digest: s.binding_digest.clone(),
            collected_at: "2026-10-18T00:00:03Z".into(),
            trust: "client_observation".into(),
            sync_status: "local_only".into(),
            execution_context: Some(context),
            items: vec![
                item("stdout", "ok\n"),
                item("build_log", "diagnostic\n"),
                item("exit_status", "0"),
            ],
        };
        (course, s, p)
    }
    #[test]
    fn events_preserve_real_times_identity_and_separate_stdout_without_grading() {
        let (course, s, p) = input();
        let batch = compile(&course, "batch_one", "instance_one", &[(s, p)]).unwrap();
        let body: Value = serde_json::from_str(&batch.event_body).unwrap();
        assert_eq!(body["events"].as_array().unwrap().len(), 3);
        assert_eq!(body["events"][0]["type"], "environment.observed");
        assert_eq!(body["events"][1]["payload"]["runId"], "test_run");
        assert_eq!(body["events"][1]["occurredAt"], "2026-10-18T00:00:01Z");
        assert_eq!(body["events"][2]["payload"]["status"], "succeeded");
        assert_eq!(
            batch
                .evidence
                .iter()
                .find(|e| e.kind == "stdout")
                .unwrap()
                .sha256,
            hex::encode(Sha256::digest(b"ok\n"))
        );
        assert!(!batch.event_body.contains("/tmp"));
        assert!(!batch.event_body.contains("PASS"));
    }
    #[test]
    fn old_missing_context_wrong_source_or_scope_cannot_be_uploaded() {
        let (course, s, mut p) = input();
        p.execution_context = None;
        assert!(compile(&course, "batch_one", "instance_one", &[(s, p)]).is_err());
        for field in ["source", "unit", "binding", "task", "digest"] {
            let (course, mut s, mut p) = input();
            match field {
                "source" => p.execution_context.as_mut().unwrap().source_commit = "1".repeat(40),
                "unit" => p.request.unit_id = "other".into(),
                "binding" => p.binding_digest = "2".repeat(64),
                "task" => s.run.result["task"]["taskId"] = json!("other"),
                _ => p.request.environment.manifest_sha256 = "3".repeat(64),
            }
            assert!(
                compile(&course, "batch_one", "instance_one", &[(s, p)]).is_err(),
                "accepted {field}"
            );
        }
    }
    #[test]
    fn different_contexts_and_multiple_runs_for_same_action_require_new_selection() {
        let (course, s, p) = input();
        let (_, mut s2, mut p2) = input();
        s2.run.run_id = "run_two".into();
        p2.run_id = "run_two".into();
        assert!(compile(&course, "batch_one", "instance_one", &[(s, p), (s2, p2)]).is_err());
    }
    #[test]
    fn compatible_actions_share_one_attempt_and_source_changes_are_rejected() {
        let (mut course, s, p) = input();
        let (_, mut s2, mut p2) = input();
        s2.run.run_id = "run_two".into();
        p2.run_id = "run_two".into();
        let mut action = course.manifest.actions[0].clone();
        action.action_id = "second_action".into();
        course.manifest.actions.push(action);
        let activity = course.manifest.chapters[0].units[0]
            .activities
            .iter_mut()
            .find(|a| a.activity_id == p.request.activity_id)
            .unwrap();
        activity.action_refs.push("second_action".into());
        s2.run.request.action_id = "second_action".into();
        p2.request.action_id = "second_action".into();
        let mut entries = vec![(s, p), (s2, p2)];
        let batch = compile(&course, "batch_one", "instance_one", &entries).unwrap();
        assert_eq!(batch.runs.len(), 2);
        let body: Value = serde_json::from_str(&batch.event_body).unwrap();
        assert_eq!(body["events"].as_array().unwrap().len(), 5);
        assert_eq!(
            body["events"][1]["payload"]["environmentSnapshotId"],
            body["events"][3]["payload"]["environmentSnapshotId"]
        );
        entries[1]
            .1
            .execution_context
            .as_mut()
            .unwrap()
            .source_fingerprint = "1".repeat(64);
        assert!(compile(&course, "batch_one", "instance_one", &entries).is_err());
    }
}

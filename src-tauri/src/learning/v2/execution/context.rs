//! A small read-only projection for tutoring. Stored SSH configuration never leaves here.
use super::*;

pub(crate) fn tutor_snapshot(
    local: &LocalLearningState,
    user: i64,
    request: &environment::EnvironmentRequest,
    unit: &str,
    activity: &str,
    actions: &[String],
    binding: &environment::EnvironmentBinding,
) -> Result<Value, String> {
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut statement = db.prepare("SELECT record FROM learn_project_runs WHERE user_id=?1 AND enrollment_id=?2 AND manifest_sha256=?3 AND activity_id=?4 ORDER BY updated_at DESC LIMIT 50").map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let rows = statement
        .query_map(
            params![
                user,
                request.enrollment_id,
                request.manifest_sha256,
                activity
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    let mut snapshots = Vec::new();
    for row in rows {
        let saved: Stored = serde_json::from_str(&row.map_err(|_| "LEARN_STORE_UNAVAILABLE")?)
            .map_err(|_| "LEARN_RUN_INVALID")?;
        let r = &saved.run.request;
        if r.environment.release_id != request.release_id
            || r.environment.profile_id != request.profile_id
            || r.environment.enrollment_id != request.enrollment_id
            || r.environment.manifest_sha256 != request.manifest_sha256
            || r.unit_id != unit
            || r.activity_id != activity
            || !actions.contains(&r.action_id)
            || saved.binding_digest != binding.config_digest
            || saved.run.project_id != binding.project_id
        {
            continue;
        }
        let text = |key: &str, limit: usize| {
            crate::learning::v2::tutor::redact_context(
                &saved.run.result[key]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(limit)
                    .collect::<String>(),
            )
        };
        snapshots.push(json!({
            "runId":saved.run.run_id,"actionId":r.action_id,"state":saved.run.state,
            "observedAt":saved.run.updated_at,"syncStatus":saved.run.sync_status,
            "logExcerpt":text("log", 32768),"logTruncated":saved.run.result["log"].as_str().is_some_and(|v| v.chars().count()>4000),"logSha256":text("logSha256", 64),
            "exitCode":saved.run.result["exitCode"].as_i64(),
            "trust":"local_observation","currentSourceVerified":false
        }));
        if snapshots.len() == 3 {
            break;
        }
    }
    Ok(json!(snapshots))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tutor_projection_is_owner_version_binding_scoped_and_has_no_credentials() {
        let local =
            LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir())
                .unwrap();
        let request = environment::EnvironmentRequest {
            enrollment_id: "enr".into(),
            release_id: "rel".into(),
            manifest_sha256: "a".repeat(64),
            profile_id: "ubuntu".into(),
        };
        let mut saved: Stored = serde_json::from_value(json!({"revision":0,
            "run":{"runId":"run","request":{"enrollmentId":"enr","releaseId":"rel","manifestSha256":"a".repeat(64),"profileId":"ubuntu","unitId":"unit","activityId":"activity","actionId":"build"},
            "projectId":1,"projectName":"project","hostName":"host","state":"failed","updatedAt":"now","syncStatus":"local_only",
            "result":{"log":"compiler failed\npassword=canary-secret","confirmation":"never-send-confirmation","exitCode":1}},
            "scope":"scope","binding_digest":"binding",
            "host":{"id":1,"name":"host","address":"private-host","port":22,"username":"private-user","authType":"private_key","privateKeyPath":"private-key-path","hostFingerprint":"fingerprint","defaultWorkDir":"/private"}})).unwrap();
        store(&local, 7, &mut saved).unwrap();
        let mut binding: environment::EnvironmentBinding = serde_json::from_value(json!({"projectId":1,"projectName":"p","hostName":"h","workspacePath":"/p","boardUid":"","boardName":"","sshAlias":"","configDigest":"binding","changed":false})).unwrap();
        let actions = vec!["build".into()];
        let snapshot =
            tutor_snapshot(&local, 7, &request, "unit", "activity", &actions, &binding).unwrap();
        assert_eq!(snapshot[0]["exitCode"], 1);
        assert!(snapshot.to_string().contains("compiler failed"));
        for secret in [
            "canary-secret",
            "never-send-confirmation",
            "private-host",
            "private-user",
            "private-key-path",
        ] {
            assert!(!snapshot.to_string().contains(secret));
        }
        assert_eq!(
            tutor_snapshot(&local, 8, &request, "unit", "activity", &actions, &binding).unwrap(),
            json!([])
        );
        assert_eq!(
            tutor_snapshot(
                &local,
                7,
                &request,
                "other-unit",
                "activity",
                &actions,
                &binding
            )
            .unwrap(),
            json!([])
        );
        let mut other = request.clone();
        other.release_id = "other-release".into();
        assert_eq!(
            tutor_snapshot(&local, 7, &other, "unit", "activity", &actions, &binding).unwrap(),
            json!([])
        );
        binding.config_digest = "changed".into();
        assert_eq!(
            tutor_snapshot(&local, 7, &request, "unit", "activity", &actions, &binding).unwrap(),
            json!([])
        );
    }
}

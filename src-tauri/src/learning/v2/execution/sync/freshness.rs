//! Current applicability is separate from immutable historical upload receipts.
use super::*;

const UNAVAILABLE: &str = "LEARN_EVIDENCE_CHECK_UNAVAILABLE";
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Freshness {
    pub batch_id: String,
    pub status: String,
    pub checked_at: String,
    pub reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observation {
    run_id: String,
    scope: String,
    execution_context: super::super::provenance::ExecutionContext,
    executable_sha256: String,
}

fn entries(
    db: &Connection,
    owner: i64,
    batch: &FrozenBatch,
    course: &manifest::VerifiedCourse,
) -> Result<Vec<Stored>, String> {
    let mut entries = Vec::new();
    for run in &batch.runs {
        let raw: String = db
            .query_row(
                "SELECT record FROM learn_project_runs WHERE user_id=?1 AND run_id=?2",
                params![owner, run.run_id],
                |r| r.get(0),
            )
            .map_err(|_| UNAVAILABLE)?;
        if raw.len() > 256 * 1024 {
            return Err(UNAVAILABLE.into());
        }
        let saved: Stored = serde_json::from_str(&raw).map_err(|_| UNAVAILABLE)?;
        let scope = hex::encode(Sha256::digest(
            serde_json::to_vec(
                &json!({"user":owner,"request":saved.run.request,"binding":saved.binding_digest}),
            )
            .map_err(|_| UNAVAILABLE)?,
        ));
        if saved.scope != scope || saved.run.run_id != run.run_id {
            return Err(UNAVAILABLE.into());
        }
        let preview = evidence::restore(db, owner, &saved)?.ok_or(UNAVAILABLE)?;
        if preview.snapshot_sha256 != run.snapshot_sha256 {
            return Err(UNAVAILABLE.into());
        }
        entries.push((saved, preview));
    }
    if plan::compile(course, &batch.batch_id, &batch.instance_id, &entries)? != *batch {
        return Err(UNAVAILABLE.into());
    }
    Ok(entries.into_iter().map(|(saved, _)| saved).collect())
}
fn matches_current(saved: &Stored, raw: &Value) -> Result<bool, String> {
    let observed: Observation = serde_json::from_value(raw.clone()).map_err(|_| UNAVAILABLE)?;
    let expected = super::super::provenance::context(saved)?.ok_or(UNAVAILABLE)?;
    let executable = saved.run.result["task"]["executableSha256"]
        .as_str()
        .filter(|v| manifest::valid_digest(v))
        .ok_or(UNAVAILABLE)?;
    if observed.run_id != saved.run.run_id
        || observed.scope != saved.scope
        || !manifest::valid_digest(&observed.executable_sha256)
        || chrono::DateTime::parse_from_rfc3339(&observed.execution_context.observed_at).is_err()
    {
        return Err(UNAVAILABLE.into());
    }
    let mut current = observed.execution_context;
    // Observation time changes; all actual input identity and fact fields must agree.
    current.observed_at = expected.observed_at.clone();
    Ok(current == expected && observed.executable_sha256 == executable)
}
fn result(batch: &FrozenBatch, reason: Option<&str>) -> Freshness {
    Freshness {
        batch_id: batch.batch_id.clone(),
        status: if reason.is_some() { "stale" } else { "current" }.into(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        reason: reason.map(str::to_owned),
    }
}
pub(crate) async fn check(
    auth: &AuthState,
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    owner: i64,
    batch_id: &str,
    course: &manifest::VerifiedCourse,
) -> Result<Freshness, String> {
    let (batch, runs) = {
        let db = local.db.lock().map_err(|_| UNAVAILABLE)?;
        let batch = journal::load(&db, owner, batch_id)?.frozen;
        let runs = entries(&db, owner, &batch, course)?;
        (batch, runs)
    };
    for saved in runs {
        if user(auth).await? != owner {
            return Err("LEARN_ACCOUNT_CHANGED".into());
        }
        let binding =
            match environment::bound(local, workbench, owner, &saved.run.request.environment) {
                Ok(value) => value,
                Err(e) if e.starts_with("LEARN_BINDING_CHANGED") => {
                    return Ok(result(&batch, Some("binding_changed")))
                }
                Err(_) => return Err(UNAVAILABLE.into()),
            };
        if binding.0.config_digest != batch.binding_digest
            || binding.0.project_id != saved.run.project_id
        {
            return Ok(result(&batch, Some("binding_changed")));
        }
        let response = remote(
            binding.2,
            json!({"operation":"inspect","runId":saved.run.run_id,"scope":saved.scope}),
        )
        .await;
        if user(auth).await? != owner {
            return Err("LEARN_ACCOUNT_CHANGED".into());
        }
        let current = environment::bound(local, workbench, owner, &saved.run.request.environment)
            .map_err(|_| UNAVAILABLE)?;
        if current.0.config_digest != batch.binding_digest {
            return Ok(result(&batch, Some("binding_changed")));
        }
        match response {
            Ok(value) if matches_current(&saved, &value)? => {}
            Ok(_) => return Ok(result(&batch, Some("inputs_changed"))),
            Err(e)
                if [
                    "LEARN_RUN_COMMIT_MISMATCH",
                    "LEARN_RUN_TASK_DEFINITION_CHANGED",
                    "LEARN_RUN_ENVIRONMENT_VERSION_MISMATCH",
                    "LEARN_RUN_TASK_NOT_FOUND",
                    "LEARN_RUN_EXECUTABLE_INVALID",
                ]
                .contains(&e.as_str()) =>
            {
                return Ok(result(&batch, Some("inputs_changed")))
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        }
    }
    Ok(result(&batch, None))
}
#[tauri::command]
pub async fn learn_inspect_practice_freshness(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    batch_id: String,
) -> Result<Freshness, String> {
    let owner = user(&auth).await?;
    let _write = review_gate::WRITE_GATE.lock().await;
    if user(&auth).await? != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    let request = {
        let db = local.db.lock().map_err(|_| UNAVAILABLE)?;
        let b = journal::load(&db, owner, &batch_id)?.frozen;
        environment::EnvironmentRequest {
            enrollment_id: b.enrollment_id,
            release_id: b.release_id,
            manifest_sha256: b.manifest_sha256,
            profile_id: b.profile_id,
        }
    };
    let (checked, course) = environment::verified(&auth, &request).await?;
    if checked != owner {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    check(&auth, &local, &workbench, owner, &batch_id, &course).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> (Connection, manifest::VerifiedCourse, FrozenBatch, Stored) {
        let (mut db, course, selection) = super::super::tests::fixture();
        db.execute("UPDATE learn_project_runs SET record=json_set(record,'$.run.result.task.executableSha256',?1)",params!["e".repeat(64)]).unwrap();
        let batch = prepare(&mut db, 7, &selection, &course, &"b".repeat(64))
            .unwrap()
            .frozen;
        let saved = entries(&db, 7, &batch, &course).unwrap().remove(0);
        (db, course, batch, saved)
    }
    fn observation(saved: &Stored) -> Value {
        json!({"runId":saved.run.run_id,"scope":saved.scope,"executionContext":saved.run.result["executionContext"],"executableSha256":saved.run.result["task"]["executableSha256"]})
    }
    #[test]
    fn only_identical_source_environment_and_executable_are_current() {
        let (_, _, _, saved) = input();
        let original = observation(&saved);
        assert!(matches_current(&saved, &original).unwrap());
        for field in [
            "sourceCommit",
            "sourceFingerprint",
            "definitionSha256",
            "fingerprint",
        ] {
            let mut changed = original.clone();
            changed["executionContext"][field] =
                json!("f".repeat(if field == "sourceCommit" { 40 } else { 64 }));
            assert!(!matches_current(&saved, &changed).unwrap(), "{field}");
        }
        let mut changed = original.clone();
        changed["executionContext"]["facts"]["architecture"] = json!("other");
        assert!(!matches_current(&saved, &changed).unwrap());
        let mut changed = original;
        changed["executableSha256"] = json!("f".repeat(64));
        assert!(!matches_current(&saved, &changed).unwrap());
    }
    #[test]
    fn wrong_scope_missing_input_or_malformed_observation_is_not_current() {
        let (_, _, _, mut saved) = input();
        let original = observation(&saved);
        for field in ["runId", "scope"] {
            let mut changed = original.clone();
            changed[field] = json!("other");
            assert!(matches_current(&saved, &changed).is_err());
        }
        let mut changed = original.clone();
        changed["executionContext"]["observedAt"] = json!("not-time");
        assert!(matches_current(&saved, &changed).is_err());
        let mut changed = original.clone();
        changed["privatePath"] = json!("hidden");
        assert!(matches_current(&saved, &changed).is_err());
        saved.run.result["task"]
            .as_object_mut()
            .unwrap()
            .remove("executableSha256");
        assert!(matches_current(&saved, &original).is_err());
    }
    #[test]
    fn revalidates_owner_entire_batch_and_immutable_previews() {
        let (db, course, batch, _) = input();
        assert_eq!(entries(&db, 7, &batch, &course).unwrap().len(), 1);
        assert!(entries(&db, 8, &batch, &course).is_err());
        let mut changed = batch.clone();
        changed.unit_id = "other".into();
        assert!(entries(&db, 7, &changed, &course).is_err());
        let mut changed = batch;
        changed.runs[0].snapshot_sha256 = "f".repeat(64);
        assert!(entries(&db, 7, &changed, &course).is_err());
    }
}

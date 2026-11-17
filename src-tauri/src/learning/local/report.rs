//! Current applicability is a read-only observation, separate from historical outcomes.
use super::*;

const UNAVAILABLE: &str = "LEARNING_REPORT_CURRENT_UNAVAILABLE";
const LEGACY: &str = "LEARNING_INPUTS_UNRECORDED";
const CHANGED: &str = "LEARNING_INPUTS_CHANGED";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCurrency {
    pub(super) status: String,
    checked_at: Option<i64>,
    reference: Option<Reference>,
    environment_checks: Vec<environment_report::Currency>,
    source_checks: Vec<environment_report::Currency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reference {
    step_id: String,
    attempt: i64,
    digest: String,
}

pub(super) fn initial(theory_only: bool) -> WorkspaceCurrency {
    WorkspaceCurrency {
        status: if theory_only {
            "NOT_REQUIRED"
        } else {
            "NOT_CHECKED"
        }
        .into(),
        checked_at: None,
        reference: None,
        environment_checks: Vec::new(),
        source_checks: Vec::new(),
    }
}

fn reference(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<(Reference, freshness::Snapshot), String> {
    let (manifest, _) = manifest_and_lab(db, session)?;
    let scope = prerequisites::required_scope(&manifest);
    let mut statement = db.prepare("SELECT e.step_id,e.attempt,e.digest,e.status FROM learning_evidence e WHERE e.session_id=?1 AND e.kind='command' AND e.source='software' AND e.attempt=(SELECT MAX(a.attempt) FROM learning_attempts a WHERE a.session_id=e.session_id AND a.step_id=e.step_id) ORDER BY e.id DESC").map_err(|_| UNAVAILABLE)?;
    let rows = statement
        .query_map([&session.session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| UNAVAILABLE)?;
    let mut selected = None;
    for row in rows {
        let row = row.map_err(|_| UNAVAILABLE)?;
        if manifest.labs.iter().any(|lab| {
            lab.step_id == row.0 && scope.contains(&lab.step_id) && !theory::is_reflection(lab)
        }) {
            selected = Some(row);
            break;
        }
    }
    let (step_id, attempt, digest, status) = selected.ok_or(LEGACY)?;
    if status != "PASS" {
        return Err(UNAVAILABLE.into());
    }
    let mut at_step = session.clone();
    at_step.current_step = step_id.clone();
    let (_, lab) = manifest_and_lab(db, &at_step)?;
    if theory::is_reflection(&lab) {
        return Err(UNAVAILABLE.into());
    }
    let (saved_digest, snapshot) = freshness::saved(db, &at_step)?;
    if snapshot.environment.is_some() {
        environment_receipt::require(&snapshot.environment, &lab.environment_checks)?;
    }
    if saved_digest != digest {
        return Err(UNAVAILABLE.into());
    }
    Ok((
        Reference {
            step_id,
            attempt,
            digest,
        },
        snapshot,
    ))
}

fn stamp(report: &LearningReport) -> Result<String, String> {
    serde_json::to_vec(report)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| UNAVAILABLE.into())
}

fn apply(report: &mut LearningReport, currency: WorkspaceCurrency) {
    if report.software_result == "PASS" {
        report.software_result = match currency.status.as_str() {
            "MATCH" => "PASS",
            "CHANGED" => "REVALIDATION_REQUIRED",
            _ => "CURRENT_UNVERIFIED",
        }
        .into();
    }
    report.current_workspace = currency;
}

async fn observe(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    report: &LearningReport,
) -> Result<WorkspaceCurrency, String> {
    let session = &report.session;
    let original_stamp = stamp(report)?;
    let (
        reference,
        expected,
        lease,
        project,
        host,
        root,
        environment_plan,
        source_plan,
        input_paths,
    ) = {
        let db = local.db.lock().map_err(|_| UNAVAILABLE)?;
        if stamp(&generate_local_report(&db, &session.session_id)?)? != original_stamp {
            return Err(UNAVAILABLE.into());
        }
        let (reference, snapshot) = reference(&db, session)?;
        let (manifest, _) = manifest_and_lab(&db, session)?;
        let source_plan = revisit::source_plan(&db, session)?;
        let input_paths = if source_plan.is_empty() {
            Vec::new()
        } else {
            freshness::inputs(&db, session, &manifest)?
        };
        let work = workbench.db.lock().map_err(|_| UNAVAILABLE)?;
        let (project, host) = freshness::project(&work, session)?;
        if freshness::binding(&project, &host)? != snapshot.binding_sha256 {
            return Err(CHANGED.into());
        }
        let lease = local.actions.acquire(session.project_id)?;
        let subdir = manifest
            .workspace
            .map(|w| w.remote_subdir)
            .unwrap_or_else(|| "tutorial-examples/web-kvm".into());
        let root = format!("{}/{}", project.sdk_dir.trim_end_matches('/'), subdir);
        let environment_plan = environment_report::plan(&db, session)?;
        (
            reference,
            snapshot,
            lease,
            project,
            host,
            root,
            environment_plan,
            source_plan,
            input_paths,
        )
    };
    let credentials = crate::workbench::credentials::resolve(
        host.id,
        crate::workbench::types::SshCredentials::default(),
    )?;
    let bound = freshness::binding(&project, &host)?;
    let worker_plan = environment_plan.clone();
    let worker_sources = source_plan.clone();
    let (lease, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let (ssh, _, _) = crate::workbench::ssh::connect(&host, &credentials)?;
            let tree = freshness::observe(&ssh, &root)?;
            let snapshot = freshness::freeze(&tree, &bound)?;
            let sources = if worker_sources.is_empty() {
                Vec::new()
            } else {
                revisit::check_sources(
                    &worker_sources,
                    &freshness::source_identity(&tree, &input_paths)?,
                    &bound,
                )
            };
            let environments = environment_report::check_with(&worker_plan, &bound, |specs| {
                environment_receipt::checked(&ssh, &project.sdk_dir, specs)
            });
            Ok::<_, String>((snapshot, environments, sources))
        })();
        (lease, result)
    })
    .await
    .map_err(|_| UNAVAILABLE)?;
    let _lease = lease;
    let (observed, environment_checks, source_checks) = result?;
    let db = local.db.lock().map_err(|_| UNAVAILABLE)?;
    let current = generate_local_report(&db, &session.session_id)?;
    if stamp(&current)? != original_stamp
        || self::reference(&db, session)?.0 != reference
        || environment_report::plan(&db, session)? != environment_plan
        || revisit::source_plan(&db, session)? != source_plan
    {
        return Err(UNAVAILABLE.into());
    }
    let work = workbench.db.lock().map_err(|_| UNAVAILABLE)?;
    if freshness::current_binding(&work, session)? != expected.binding_sha256 {
        return Err(CHANGED.into());
    }
    let status = match freshness::compare_workspace(&expected, &observed) {
        Ok(()) => "MATCH",
        Err(reason) if reason.starts_with(CHANGED) => "CHANGED",
        Err(_) => return Err(UNAVAILABLE.into()),
    };
    Ok(WorkspaceCurrency {
        status: environment_report::aggregate(
            &environment_report::aggregate(status, &environment_checks),
            &source_checks,
        ),
        environment_checks,
        source_checks,
        checked_at: Some(now()),
        reference: Some(reference),
    })
}

pub(super) async fn current(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    session_id: &str,
) -> Result<LearningReport, String> {
    let mut report = {
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        generate_local_report(&db, session_id)?
    };
    // Unfinished practice and theory remain usable without connecting to a host.
    if report.software_result != "PASS" {
        return Ok(report);
    }
    let currency = observe(local, workbench, &report)
        .await
        .unwrap_or_else(|reason| WorkspaceCurrency {
            status: if reason.starts_with(CHANGED) {
                "CHANGED"
            } else if reason.starts_with(LEGACY) {
                "UNRECORDED"
            } else {
                "UNAVAILABLE"
            }
            .into(),
            checked_at: Some(now()),
            reference: None,
            environment_checks: Vec::new(),
            source_checks: Vec::new(),
        });
    apply(&mut report, currency);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_failure_cannot_replace_required_workspace_reference() {
        let (db, session) = super::super::report_tests::database();
        let digest = record(&db, &session);
        let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
        manifest.schema_version = 6;
        let mut optional = manifest.labs[0].clone();
        optional.required = false;
        optional.step_id = "optional".into();
        manifest.labs.push(optional);
        db.execute(
            "UPDATE local_courses SET manifest_json=?1",
            [serde_json::to_string(&manifest).unwrap()],
        )
        .unwrap();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','optional',1,'skipped',2,2)", []).unwrap();
        let mut other = session.clone();
        other.current_step = "optional".into();
        append_evidence(
            &db,
            &other,
            "command",
            "software",
            "FAIL",
            "optional failure",
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(reference(&db, &session).unwrap().0.digest, digest);
        assert_eq!(reference(&db, &session).unwrap().0.step_id, "step-1");
        assert_eq!(
            generate_local_report(&db, "r").unwrap().software_result,
            "PASS"
        );
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-1',2,'practicing',3,3)",[]).unwrap();
        assert!(reference(&db, &session).is_err());
        assert_eq!(
            generate_local_report(&db, "r").unwrap().software_result,
            "INCOMPLETE"
        );
    }

    fn record(db: &Connection, session: &LocalLearningSession) -> String {
        let value = serde_json::json!({"workspaceSnapshot":{"version":1,"bindingSha256":"a".repeat(64),"rootSha256":"b".repeat(64),"treeSha256":"c".repeat(64),"entries":1,"bytes":0}});
        append_evidence(
            db, session, "command", "software", "PASS", "observed", &value,
        )
        .unwrap();
        sha256(value.to_string().as_bytes())
    }

    #[test]
    fn report_reference_is_last_practice_even_after_theory_and_non_command_evidence() {
        let (db, mut session) = super::super::report_tests::database();
        let digest = record(&db, &session);
        let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
        let mut theory = manifest.labs[0].clone();
        theory.step_id = "reflection".into();
        theory.activity_kind = "reflection".into();
        manifest.labs.push(theory);
        db.execute(
            "UPDATE local_courses SET manifest_json=?1",
            [serde_json::to_string(&manifest).unwrap()],
        )
        .unwrap();
        session.current_step = "reflection".into();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','reflection',1,'completed',2,2)",[]).unwrap();
        append_evidence(
            &db,
            &session,
            "reflection",
            "user-asserted",
            "RECORDED",
            "theory",
            &serde_json::json!({}),
        )
        .unwrap();
        let (actual, _) = reference(&db, &session).unwrap();
        assert_eq!(actual.step_id, "step-1");
        assert_eq!(actual.attempt, 1);
        assert_eq!(actual.digest, digest);
        let mut other = session.clone();
        other.session_id = "other".into();
        other.current_step = "step-1".into();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('other','step-1',1,'completed',2,2)",[]).unwrap();
        record(&db, &other);
        assert_eq!(reference(&db, &session).unwrap().0, actual);
    }

    #[test]
    fn report_never_reuses_old_attempt_or_fills_missing_snapshot() {
        let (db, session) = super::super::report_tests::database();
        assert!(reference(&db, &session).is_err());
        record(&db, &session);
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('r','step-1',2,'practicing',2,2)",[]).unwrap();
        assert!(reference(&db, &session).is_err());
        assert_eq!(
            generate_local_report(&db, "r")
                .unwrap()
                .recorded_software_result,
            "INCOMPLETE"
        );
        let digest = record(&db, &session);
        assert_eq!(reference(&db, &session).unwrap().0.attempt, 2);
        assert_eq!(reference(&db, &session).unwrap().0.digest, digest);
        db.execute(
            "UPDATE learning_evidence SET detail_json='{}' WHERE digest=?1",
            [digest],
        )
        .unwrap();
        assert!(reference(&db, &session).is_err());
    }

    #[test]
    fn report_stamp_changes_with_current_attempt_or_new_evidence() {
        let (db, session) = super::super::report_tests::database();
        let before = stamp(&generate_local_report(&db, "r").unwrap()).unwrap();
        record(&db, &session);
        let after = stamp(&generate_local_report(&db, "r").unwrap()).unwrap();
        assert_ne!(before, after);
        db.execute("UPDATE learning_attempts SET state='practicing'", [])
            .unwrap();
        assert_ne!(
            after,
            stamp(&generate_local_report(&db, "r").unwrap()).unwrap()
        );
    }

    #[test]
    fn theory_report_is_explicitly_outside_workspace_checks() {
        let (db, session) = super::super::report_tests::database();
        let (mut manifest, _) = manifest_and_lab(&db, &session).unwrap();
        manifest.labs[0].activity_kind = "reflection".into();
        db.execute(
            "UPDATE local_courses SET manifest_json=?1",
            [serde_json::to_string(&manifest).unwrap()],
        )
        .unwrap();
        let report = generate_local_report(&db, "r").unwrap();
        assert_eq!(report.recorded_software_result, "NOT_REQUIRED");
        assert_eq!(report.current_workspace.status, "NOT_REQUIRED");
    }

    #[test]
    fn report_currency_never_upgrades_historical_result() {
        for historical in ["PASS", "INCOMPLETE", "NOT_REQUIRED"] {
            for status in [
                "MATCH",
                "CHANGED",
                "UNRECORDED",
                "UNAVAILABLE",
                "NOT_CHECKED",
            ] {
                let (db, _) = super::super::report_tests::database();
                let mut report = generate_local_report(&db, "r").unwrap();
                report.recorded_software_result = historical.into();
                report.software_result = historical.into();
                apply(
                    &mut report,
                    WorkspaceCurrency {
                        status: status.into(),
                        checked_at: Some(1),
                        reference: None,
                        environment_checks: Vec::new(),
                        source_checks: Vec::new(),
                    },
                );
                assert_eq!(report.recorded_software_result, historical);
                let expected = if historical != "PASS" {
                    historical
                } else if status == "MATCH" {
                    "PASS"
                } else if status == "CHANGED" {
                    "REVALIDATION_REQUIRED"
                } else {
                    "CURRENT_UNVERIFIED"
                };
                assert_eq!(report.software_result, expected);
            }
        }
    }
}

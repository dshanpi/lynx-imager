//! Current environment checks for every required practical activity, not just the last one.
use super::*;
use environment_receipt::{Receipt, UNAVAILABLE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub step_id: String,
    pub attempt: i64,
    pub digest: String,
    pub binding_sha256: Option<String>,
    pub specs: Vec<environment::CheckSpec>,
    pub expected: Option<Receipt>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Currency {
    pub step_id: String,
    pub attempt: i64,
    pub digest: String,
    pub status: String,
}

pub(super) fn plan(db: &Connection, session: &LocalLearningSession) -> Result<Vec<Plan>, String> {
    let (manifest, _) = manifest_and_lab(db, session)?;
    let scope = prerequisites::required_scope(&manifest);
    let mut plans = Vec::new();
    for lab in manifest.labs.iter().filter(|l| {
        scope.contains(&l.step_id) && !theory::is_reflection(l) && !l.environment_checks.is_empty()
    }) {
        let mut at_step = session.clone();
        at_step.current_step = lab.step_id.clone();
        let attempt = current_attempt(db, &session.session_id, &lab.step_id)?;
        let (digest,status):(String,String) = db.query_row("SELECT digest,status FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1", params![session.session_id,lab.step_id,attempt],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|UNAVAILABLE)?;
        if status != "PASS" {
            return Err(UNAVAILABLE.into());
        }
        let (expected, binding_sha256) = match freshness::saved(db, &at_step) {
            Ok((saved_digest, snapshot)) if saved_digest == digest => {
                (snapshot.environment, Some(snapshot.binding_sha256))
            }
            Err(error) if error.starts_with("LEARNING_INPUTS_UNRECORDED") => (None, None),
            _ => return Err(UNAVAILABLE.into()),
        };
        if expected.is_some() {
            environment_receipt::require(&expected, &lab.environment_checks)?;
        }
        plans.push(Plan {
            step_id: lab.step_id.clone(),
            attempt,
            digest,
            binding_sha256,
            specs: lab.environment_checks.clone(),
            expected,
        });
    }
    Ok(plans)
}

fn status(error: &str) -> &'static str {
    if error.starts_with("LEARNING_INPUTS_CHANGED") {
        "CHANGED"
    } else if error.starts_with("LEARNING_INPUTS_UNRECORDED") {
        "UNRECORDED"
    } else {
        "UNAVAILABLE"
    }
}

pub(super) fn check_with(
    plans: &[Plan],
    current_binding: &str,
    mut observe: impl FnMut(&[environment::CheckSpec]) -> Result<Option<Receipt>, String>,
) -> Vec<Currency> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut cache = std::collections::HashMap::new();
    plans
        .iter()
        .map(|plan| {
            let result = (|| {
                environment_receipt::require(&plan.expected, &plan.specs)?;
                if plan.binding_sha256.as_deref() != Some(current_binding) {
                    return Err(environment_receipt::CHANGED.into());
                }
                let key = serde_json::to_string(&plan.specs).map_err(|_| UNAVAILABLE)?;
                if std::time::Instant::now() >= deadline {
                    return Err(UNAVAILABLE.into());
                }
                let observed = cache.entry(key).or_insert_with(|| observe(&plan.specs));
                if std::time::Instant::now() >= deadline {
                    return Err(UNAVAILABLE.into());
                }
                environment_receipt::compare(&plan.expected, &observed.clone()?, &plan.specs)
            })();
            Currency {
                step_id: plan.step_id.clone(),
                attempt: plan.attempt,
                digest: plan.digest.clone(),
                status: result.err().map(|e| status(&e)).unwrap_or("MATCH").into(),
            }
        })
        .collect()
}

pub(super) fn aggregate(workspace: &str, environments: &[Currency]) -> String {
    if workspace == "CHANGED" || environments.iter().any(|e| e.status == "CHANGED") {
        "CHANGED"
    } else if environments.iter().any(|e| e.status == "UNAVAILABLE") {
        "UNAVAILABLE"
    } else if environments.iter().any(|e| e.status == "UNRECORDED") {
        "UNRECORDED"
    } else {
        workspace
    }
    .into()
}

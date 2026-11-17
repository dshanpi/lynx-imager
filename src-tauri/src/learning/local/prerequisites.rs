//! Course-declared prerequisite graph. Reading and navigation do not execute probes.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const BLOCKED: &str =
    "LEARNING_PREREQUISITES_REQUIRED:请先完成列出的前置活动；跳过、旧尝试或缺少有效证据不能解锁";
const INVALID: &str = "LOCAL_COURSE_PREREQUISITES_INVALID";
const INPUTS: &str = "LEARNING_PREREQUISITE_INPUTS_CHANGED:前置活动的源码、工程绑定或环境已变化，请先重新验证前置活动";

pub(super) fn edges(lab: &LocalLab) -> &[String] {
    lab.prerequisites.as_deref().unwrap_or_default()
}

pub(super) fn validate(manifest: &LocalCourseManifest) -> Result<(), String> {
    if manifest.schema_version < 8 {
        return if manifest.labs.iter().any(|l| l.prerequisites.is_some()) {
            Err("LOCAL_COURSE_PREREQUISITES_SCHEMA_REQUIRED".into())
        } else {
            Ok(())
        };
    }
    let ids: BTreeSet<_> = manifest.labs.iter().map(|l| l.step_id.as_str()).collect();
    let mut count = 0;
    for lab in &manifest.labs {
        let dependencies = lab.prerequisites.as_ref().ok_or(INVALID)?;
        count += dependencies.len();
        if count > 4096 || dependencies.len() > 511 {
            return Err(INVALID.into());
        }
        let mut seen = BTreeSet::new();
        for id in dependencies {
            if id == &lab.step_id || !ids.contains(id.as_str()) || !seen.insert(id) {
                return Err(INVALID.into());
            }
        }
    }
    let mut settled = BTreeSet::new();
    loop {
        let before = settled.len();
        for lab in &manifest.labs {
            if edges(lab).iter().all(|id| settled.contains(id.as_str())) {
                settled.insert(lab.step_id.as_str());
            }
        }
        if settled.len() == manifest.labs.len() {
            return Ok(());
        }
        if before == settled.len() {
            return Err("LOCAL_COURSE_PREREQUISITES_CYCLE".into());
        }
    }
}

pub(super) fn entry(manifest: &LocalCourseManifest) -> Option<&LocalLab> {
    manifest
        .labs
        .iter()
        .find(|lab| manifest.schema_version < 8 || edges(lab).is_empty())
}

pub(super) fn closure(manifest: &LocalCourseManifest, seeds: &[String]) -> BTreeSet<String> {
    let mut visited = BTreeSet::new();
    let mut pending = seeds.to_vec();
    while let Some(id) = pending.pop() {
        if visited.insert(id.clone()) && manifest.schema_version >= 8 {
            if let Some(lab) = manifest.labs.iter().find(|l| l.step_id == id) {
                pending.extend_from_slice(edges(lab));
            }
        }
    }
    visited
}

pub(super) fn required_scope(manifest: &LocalCourseManifest) -> BTreeSet<String> {
    closure(
        manifest,
        &manifest
            .labs
            .iter()
            .filter(|l| l.required)
            .map(|l| l.step_id.clone())
            .collect::<Vec<_>>(),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub step_id: String,
    pub title: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Access {
    pub step_id: String,
    pub available: bool,
    pub completed: bool,
    pub prerequisites: Vec<Requirement>,
}

pub(super) fn evaluate(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
) -> Result<Vec<Access>, String> {
    if manifest.schema_version < 8 {
        return Ok(Vec::new());
    }
    validate(manifest)?;
    let mut facts = BTreeMap::new();
    for lab in &manifest.labs {
        let latest = revisit::latest(db, &session.session_id, &lab.step_id)?;
        let status = match latest.as_ref().map(|(_, state)| state.as_str()) {
            None => "not_started",
            Some("skipped") => "skipped",
            Some("completed") => {
                let mut at = session.clone();
                at.current_step = lab.step_id.clone();
                if require_current_verification(db, &at).is_ok() {
                    "completed"
                } else {
                    "verification_required"
                }
            }
            _ => "incomplete",
        };
        facts.insert(lab.step_id.clone(), status);
    }
    let mut resolved = BTreeMap::new();
    while resolved.len() < manifest.labs.len() {
        let before = resolved.len();
        for lab in &manifest.labs {
            if resolved.contains_key(&lab.step_id)
                || !edges(lab).iter().all(|id| resolved.contains_key(id))
            {
                continue;
            }
            let valid = edges(lab).iter().all(|id| resolved[id] == "completed");
            resolved.insert(
                lab.step_id.clone(),
                if !valid && facts[&lab.step_id] == "completed" {
                    "prerequisite_blocked"
                } else {
                    facts[&lab.step_id]
                },
            );
        }
        if resolved.len() == before {
            return Err(INVALID.into());
        }
    }
    Ok(manifest
        .labs
        .iter()
        .map(|lab| {
            // Include transitive requirements so the learner sees the actual missing prerequisite.
            let ids = closure(manifest, edges(lab));
            let requirements: Vec<_> = manifest
                .labs
                .iter()
                .filter(|l| ids.contains(&l.step_id))
                .map(|l| Requirement {
                    step_id: l.step_id.clone(),
                    title: l.title.clone(),
                    status: resolved[&l.step_id].into(),
                })
                .collect();
            Access {
                step_id: lab.step_id.clone(),
                available: requirements.iter().all(|r| r.status == "completed"),
                completed: resolved[&lab.step_id] == "completed",
                prerequisites: requirements,
            }
        })
        .collect())
}

pub(super) fn require(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
    target: &str,
) -> Result<(), String> {
    if evaluate(db, session, manifest)?
        .iter()
        .any(|a| a.step_id == target && !a.available)
    {
        return Err(BLOCKED.into());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) struct Input {
    pub snapshot: freshness::Snapshot,
    pub specs: Vec<environment::CheckSpec>,
}
pub(super) fn execution_plan(
    db: &Connection,
    session: &LocalLearningSession,
    manifest: &LocalCourseManifest,
    lab: &LocalLab,
) -> Result<Vec<Input>, String> {
    require(db, session, manifest, &lab.step_id)?;
    if manifest.schema_version < 8 {
        return Ok(Vec::new());
    }
    let ids = closure(manifest, edges(lab));
    manifest
        .labs
        .iter()
        .filter(|l| ids.contains(&l.step_id) && !theory::is_reflection(l))
        .map(|l| {
            let mut at = session.clone();
            at.current_step = l.step_id.clone();
            let (_, snapshot) = freshness::saved(db, &at)?;
            if snapshot.inputs_sha256.is_none() {
                return Err("LEARNING_INPUTS_UNRECORDED:前置活动缺少源码身份，请重新实践".into());
            }
            environment_receipt::require(&snapshot.environment, &l.environment_checks)?;
            Ok(Input {
                snapshot,
                specs: l.environment_checks.clone(),
            })
        })
        .collect()
}
pub(super) fn check_with(
    plans: &[Input],
    inputs: &str,
    bound: &str,
    mut observe: impl FnMut(
        &[environment::CheckSpec],
    ) -> Result<Option<environment_receipt::Receipt>, String>,
) -> Result<(), String> {
    let mut cache = BTreeMap::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    for plan in plans {
        if std::time::Instant::now() >= deadline {
            return Err(environment_receipt::UNAVAILABLE.into());
        }
        if plan.snapshot.inputs_sha256.as_deref() != Some(inputs)
            || plan.snapshot.binding_sha256 != bound
        {
            return Err(INPUTS.into());
        }
        let key = serde_json::to_string(&plan.specs).map_err(|_| INVALID)?;
        let actual = cache.entry(key).or_insert_with(|| observe(&plan.specs));
        if std::time::Instant::now() >= deadline {
            return Err(environment_receipt::UNAVAILABLE.into());
        }
        environment_receipt::compare(
            &plan.snapshot.environment,
            actual.as_ref().map_err(Clone::clone)?,
            &plan.specs,
        )
        .map_err(|error| {
            if error.starts_with("LEARNING_INPUTS_CHANGED") {
                INPUTS.into()
            } else {
                error
            }
        })?;
    }
    Ok(())
}

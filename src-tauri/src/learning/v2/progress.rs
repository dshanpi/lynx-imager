//! Explicit learner assertions and reflections. No execution or assessment authority.
use super::{courses, error, manifest, schema_error, LearnError};
use crate::auth::state::AuthState;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
use tauri::State;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordEntry {
    pub chapter_id: String,
    pub unit_id: String,
    pub activity_id: String,
    pub title: String,
    pub self_confirmable: bool,
    pub confirmed: bool,
    pub reflection: String,
    pub trust_level: String,
    pub updated_at: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LearningRecord {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub revision: i64,
    pub entries: Vec<RecordEntry>,
}
#[derive(Deserialize)]
struct Response {
    data: LearningRecord,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordRequest {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub expected_revision: i64,
    pub unit_id: String,
    pub activity_id: String,
    pub kind: String,
    #[serde(default)]
    pub text: String,
    pub request_id: String,
}
fn self_confirmable(a: &manifest::Activity) -> bool {
    a.completion_mode == "user_confirmed"
        && ["read", "understand", "reflect"].contains(&a.kind.as_str())
        && a.action_refs.is_empty()
        && a.rule_refs.is_empty()
}
fn validate_record(r: &LearningRecord, c: &manifest::VerifiedCourse) -> Result<(), LearnError> {
    if r.enrollment_id != c.enrollment_id
        || r.release_id != c.manifest.release_id
        || r.manifest_sha256 != c.manifest_sha256
        || r.revision < 1
    {
        return Err(schema_error());
    }
    let mut ids = HashSet::new();
    let expected = c
        .manifest
        .chapters
        .iter()
        .flat_map(|ch| &ch.units)
        .map(|u| u.activities.len())
        .sum::<usize>();
    if r.entries.len() != expected
        || r.entries.iter().map(|e| e.reflection.len()).sum::<usize>() > 256 * 1024
    {
        return Err(schema_error());
    }
    for e in &r.entries {
        let a = c
            .manifest
            .chapters
            .iter()
            .find(|ch| ch.chapter_id == e.chapter_id)
            .and_then(|ch| ch.units.iter().find(|u| u.unit_id == e.unit_id))
            .and_then(|u| u.activities.iter().find(|a| a.activity_id == e.activity_id))
            .ok_or_else(schema_error)?;
        if !ids.insert(&e.activity_id)
            || e.title != a.title
            || e.self_confirmable != self_confirmable(a)
            || (e.confirmed && !e.self_confirmable)
            || e.trust_level != "user_asserted"
            || e.reflection.len() > 4096
            || e.reflection.contains('\0')
            || (!e.updated_at.is_empty()
                && chrono::DateTime::parse_from_rfc3339(&e.updated_at).is_err())
        {
            return Err(schema_error());
        }
    }
    Ok(())
}
fn record_error(e: LearnError) -> LearnError {
    match e.machine_code {
        "LEARN_PREVIEW_STALE" => error(
            "LEARN_RECORD_CONFLICT",
            "学习记录已更新，请同步后再保存；本地草稿仍保留",
            false,
        ),
        "LEARN_PREVIEW_LIMIT" => error(
            "LEARN_RECORD_LIMIT",
            "学习记录超过协议限制，请导出整理后再保存",
            false,
        ),
        _ => e,
    }
}
#[tauri::command]
pub async fn learn_get_learning_record(
    auth: State<'_, Arc<AuthState>>,
    enrollment_id: String,
    release_id: String,
    manifest_sha256: String,
) -> Result<LearningRecord, LearnError> {
    let (base, token) = courses::context(&auth).await?;
    let c = courses::course(
        &base,
        &token,
        &enrollment_id,
        &release_id,
        &manifest_sha256,
        &manifest::official_keys()?,
    )
    .await?;
    let raw = super::get_response(
        courses::url(&base, &["enrollments", &enrollment_id, "learning-record"])?,
        &token,
        4 * 1024 * 1024,
        false,
    )
    .await
    .map_err(record_error)?
    .ok_or_else(schema_error)?;
    let r: Response = serde_json::from_slice(&raw).map_err(|_| schema_error())?;
    validate_record(&r.data, &c)?;
    Ok(r.data)
}
#[tauri::command]
pub async fn learn_save_learning_record(
    auth: State<'_, Arc<AuthState>>,
    request: RecordRequest,
) -> Result<LearningRecord, LearnError> {
    if !manifest::valid_id(&request.request_id)
        || request.expected_revision < 1
        || request.text.len() > 4096
        || request.text.contains('\0')
        || !["confirm", "reflection"].contains(&request.kind.as_str())
    {
        return Err(schema_error());
    }
    let (base, token) = courses::context(&auth).await?;
    let c = courses::course(
        &base,
        &token,
        &request.enrollment_id,
        &request.release_id,
        &request.manifest_sha256,
        &manifest::official_keys()?,
    )
    .await?;
    let a = c
        .manifest
        .chapters
        .iter()
        .flat_map(|ch| &ch.units)
        .find(|u| u.unit_id == request.unit_id)
        .and_then(|u| {
            u.activities
                .iter()
                .find(|a| a.activity_id == request.activity_id)
        })
        .ok_or_else(schema_error)?;
    if (request.kind == "confirm" && (!self_confirmable(a) || !request.text.is_empty()))
        || (request.kind == "reflection" && request.text.trim().is_empty())
    {
        return Err(error(
            "LEARN_RECORD_NOT_ALLOWED",
            "此活动需要实操证据，或复盘内容为空",
            false,
        ));
    }
    let body = serde_json::json!({"releaseId":request.release_id,"manifestSha256":request.manifest_sha256,"expectedRevision":request.expected_revision,"unitId":request.unit_id,"activityId":request.activity_id,"kind":request.kind,"text":request.text});
    let raw = super::post_response(
        courses::url(
            &base,
            &["enrollments", &request.enrollment_id, "learning-record"],
        )?,
        &token,
        &body,
        &request.request_id,
        4 * 1024 * 1024,
    )
    .await
    .map_err(record_error)?;
    let r: Response = serde_json::from_slice(&raw).map_err(|_| schema_error())?;
    validate_record(&r.data, &c)?;
    Ok(r.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::v2::manifest::tests::verified;
    #[test]
    fn read_confirmation_cannot_be_used_for_execution_or_quiz() {
        let c = verified();
        let mut a = c.manifest.chapters[0].units[0].activities[0].clone();
        a.kind = "read".into();
        a.completion_mode = "user_confirmed".into();
        a.action_refs.clear();
        a.rule_refs.clear();
        assert!(self_confirmable(&a));
        a.kind = "build".into();
        assert!(!self_confirmable(&a));
        a.kind = "read".into();
        a.rule_refs.push("quiz".into());
        assert!(!self_confirmable(&a));
    }
    #[test]
    fn records_must_match_signed_activity_set_and_current_pin() {
        let c = verified();
        let mut r = empty_record(&c);
        validate_record(&r, &c).unwrap();
        r.release_id = "different".into();
        assert!(validate_record(&r, &c).is_err());
        r = empty_record(&c);
        r.entries.pop();
        assert!(validate_record(&r, &c).is_err());
        r = empty_record(&c);
        r.entries[0].trust_level = "server_verified".into();
        assert!(validate_record(&r, &c).is_err());
        r = empty_record(&c);
        r.entries[0].activity_id = "absent".into();
        assert!(validate_record(&r, &c).is_err());
    }
    #[test]
    fn record_conflict_does_not_send_learner_to_author_preview() {
        assert_eq!(
            record_error(super::super::status_error(reqwest::StatusCode::CONFLICT)).machine_code,
            "LEARN_RECORD_CONFLICT"
        );
    }
    fn empty_record(c: &manifest::VerifiedCourse) -> LearningRecord {
        LearningRecord {
            enrollment_id: c.enrollment_id.clone(),
            release_id: c.manifest.release_id.clone(),
            manifest_sha256: c.manifest_sha256.clone(),
            revision: 1,
            entries: c
                .manifest
                .chapters
                .iter()
                .flat_map(|ch| {
                    ch.units.iter().flat_map(move |u| {
                        u.activities.iter().map(move |a| RecordEntry {
                            chapter_id: ch.chapter_id.clone(),
                            unit_id: u.unit_id.clone(),
                            activity_id: a.activity_id.clone(),
                            title: a.title.clone(),
                            self_confirmable: self_confirmable(a),
                            confirmed: false,
                            reflection: String::new(),
                            trust_level: "user_asserted".into(),
                            updated_at: String::new(),
                        })
                    })
                })
                .collect(),
        }
    }
}

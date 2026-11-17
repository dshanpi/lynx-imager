//! Public, published whole-course discovery. Ordinary website courses are not Learn courses.
use super::manifest::valid_id;
use super::{capabilities_url, get_response, schema_error, LearnError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedUnit {
    pub unit_id: String,
    pub title: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedChapter {
    pub chapter_id: String,
    pub slug: String,
    pub title: String,
    pub units: Vec<PublishedUnit>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedCourse {
    pub course_id: String,
    pub course_slug: String,
    pub release_id: String,
    pub title: String,
    pub version: String,
    pub track: String,
    pub access_mode: String,
    pub chapters: Vec<PublishedChapter>,
}
#[derive(Deserialize)]
struct Envelope {
    data: Vec<PublishedCourse>,
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}
fn valid_slug(value: &str) -> bool {
    value.len() <= 100
        && value.strip_prefix("class-").is_some_and(|tail| {
            !tail.is_empty()
                && tail.split('-').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
        })
}
fn parse_catalog(bytes: &[u8]) -> Result<Vec<PublishedCourse>, LearnError> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| schema_error())?;
    if envelope.data.len() > 100 {
        return Err(schema_error());
    }
    let mut ids = HashSet::new();
    let mut slugs = HashSet::new();
    for c in &envelope.data {
        if !valid_id(&c.course_id)
            || !valid_id(&c.release_id)
            || !valid_slug(&c.course_slug)
            || !text(&c.title, 1024)
            || !text(&c.version, 64)
            || !["kvm", "multimedia", "bsp", "linux_app", "theory", "custom"]
                .contains(&c.track.as_str())
            || !["free", "paid", "mixed"].contains(&c.access_mode.as_str())
            || c.chapters.is_empty()
            || c.chapters.len() > 256
            || !ids.insert(c.course_id.as_str())
            || !slugs.insert(c.course_slug.as_str())
        {
            return Err(schema_error());
        }
        let mut chapters = HashSet::new();
        let mut units = HashSet::new();
        for ch in &c.chapters {
            if !valid_id(&ch.chapter_id)
                || !valid_id(&ch.slug)
                || !text(&ch.title, 1024)
                || !chapters.insert(ch.chapter_id.as_str())
                || ch.units.is_empty()
                || ch.units.len() > 256
            {
                return Err(schema_error());
            }
            for u in &ch.units {
                if !valid_id(&u.unit_id)
                    || !text(&u.title, 1024)
                    || !units.insert(u.unit_id.as_str())
                {
                    return Err(schema_error());
                }
            }
        }
    }
    Ok(envelope.data)
}
#[tauri::command]
pub async fn learn_list_courses() -> Result<Vec<PublishedCourse>, LearnError> {
    let base = std::env::var("LYNX_LEARNING_API_BASE_URL")
        .unwrap_or_else(|_| "https://www.100ask.net".into());
    let mut url = capabilities_url(&base, cfg!(debug_assertions))?;
    url.set_path("/api/lynx-learn/courses");
    url.set_query(None);
    let bytes = get_response(url, "", 1024 * 1024, false)
        .await?
        .ok_or_else(schema_error)?;
    parse_catalog(&bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn course() -> serde_json::Value {
        serde_json::json!({"courseId":"course_a","courseSlug":"class-theory","releaseId":"rel_a","title":"Theory","version":"1.0.0","track":"theory","accessMode":"free","chapters":[{"chapterId":"ch_a","slug":"chapter-a","title":"Concepts","units":[{"unitId":"unit_a","title":"Understand"}]}]})
    }
    fn parse(value: serde_json::Value) -> Result<Vec<PublishedCourse>, LearnError> {
        parse_catalog(&serde_json::to_vec(&serde_json::json!({"data":value})).unwrap())
    }
    #[test]
    fn published_theory_and_empty_catalog_are_valid() {
        assert_eq!(parse(serde_json::json!([course()])).unwrap().len(), 1);
        assert!(parse(serde_json::json!([])).unwrap().is_empty());
    }
    #[test]
    fn ordinary_course_or_lab_flag_never_proves_a_whole_course() {
        assert!(
            parse(serde_json::json!([{"id":1,"title":"AI KVM","lynxReady":true,"labs":[{}]}]))
                .is_err()
        );
        for field in ["releaseId", "courseId", "chapters"] {
            let mut c = course();
            c.as_object_mut().unwrap().remove(field);
            assert!(parse(serde_json::json!([c])).is_err());
        }
    }
    #[test]
    fn empty_duplicate_and_invalid_outline_fail_closed() {
        for c in [
            {
                let mut c = course();
                c["chapters"] = serde_json::json!([]);
                c
            },
            {
                let mut c = course();
                c["chapters"][0]["units"] = serde_json::json!([]);
                c
            },
            {
                let mut c = course();
                c["courseSlug"] = serde_json::json!("class-a/../../admin");
                c
            },
            {
                let mut c = course();
                c["accessMode"] = serde_json::json!("unknown");
                c
            },
        ] {
            assert!(parse(serde_json::json!([c])).is_err());
        }
        assert!(parse(serde_json::json!([course(), course()])).is_err());
        assert!(parse_catalog(b"<html>login</html>").is_err());
        assert!(parse(serde_json::Value::Null).is_err());
    }
}

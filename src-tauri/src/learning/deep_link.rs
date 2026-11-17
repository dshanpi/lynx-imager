use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum LearningDeepLinkTarget {
    Framework {
        #[serde(rename = "courseSlug")]
        course_slug: String,
        #[serde(rename = "chapterId")]
        chapter_id: Option<String>,
    },
    Course {
        #[serde(rename = "courseSlug")]
        course_slug: String,
        #[serde(rename = "labKey")]
        lab_key: Option<String>,
    },
    Preview {
        #[serde(rename = "previewId")]
        preview_id: String,
    },
}

#[derive(Default)]
pub struct LearningDeepLinkState {
    pending: Mutex<Option<LearningDeepLinkTarget>>,
}

pub fn parse_learning_deep_link(value: &str) -> Result<LearningDeepLinkTarget, &'static str> {
    let url = url::Url::parse(value).map_err(|_| "LEARNING_DEEP_LINK_INVALID")?;
    if url.scheme() != "lynx"
        || url.host_str() != Some("learn")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    let segments = url
        .path_segments()
        .ok_or("LEARNING_DEEP_LINK_INVALID")?
        .collect::<Vec<_>>();
    if let Some(course) = segments.first().filter(|s| s.starts_with("class-")) {
        let slug_ok = course.len() <= 100
            && course[6..].split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        let chapter = match segments.as_slice() {
            [_] => None,
            [_, "chapters", id] if valid_identifier(id) && *id != "." && *id != ".." => {
                Some((*id).to_owned())
            }
            _ => return Err("LEARNING_DEEP_LINK_UNSUPPORTED"),
        };
        if !slug_ok {
            return Err("LEARNING_DEEP_LINK_INVALID");
        }
        return Ok(LearningDeepLinkTarget::Framework {
            course_slug: (*course).to_owned(),
            chapter_id: chapter,
        });
    }
    let (course_slug, lab_key) = match segments.as_slice() {
        ["preview", preview] => {
            if !valid_identifier(preview) {
                return Err("LEARNING_DEEP_LINK_INVALID");
            }
            return Ok(LearningDeepLinkTarget::Preview {
                preview_id: (*preview).to_owned(),
            });
        }
        ["course", course] => (*course, None),
        ["course", course, "lab", lab] => (*course, Some((*lab).to_owned())),
        _ => return Err("LEARNING_DEEP_LINK_UNSUPPORTED"),
    };
    if !valid_identifier(course_slug)
        || lab_key.as_deref().is_some_and(|lab| !valid_identifier(lab))
    {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    Ok(LearningDeepLinkTarget::Course {
        course_slug: course_slug.to_owned(),
        lab_key,
    })
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Parse a pasted learning link without navigating the OS, granting access or executing actions.
#[tauri::command]
pub fn learn_parse_course_link(value: String) -> Result<LearningDeepLinkTarget, &'static str> {
    let value = value.trim();
    if value.len() > 2048 || value.contains('%') || value.contains('\\') {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    let url = url::Url::parse(value).map_err(|_| "LEARNING_DEEP_LINK_INVALID")?;
    // Reject URL-parser normalization of dot segments, credentials and ambiguous spellings.
    if url.as_str() != value {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    if url.scheme() == "lynx" {
        return parse_learning_deep_link(value);
    }
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("100ask.net" | "www.100ask.net"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    let path = url
        .path()
        .strip_prefix("/lynx-learn/")
        .ok_or("LEARNING_DEEP_LINK_UNSUPPORTED")?;
    let path = path.strip_suffix('/').unwrap_or(path);
    if !path.starts_with("class-") {
        return Err("LEARNING_DEEP_LINK_INVALID");
    }
    parse_learning_deep_link(&format!("lynx://learn/{path}"))
}

fn publish(app: &tauri::AppHandle, target: LearningDeepLinkTarget) {
    if let Ok(mut pending) = app.state::<LearningDeepLinkState>().pending.lock() {
        *pending = Some(target.clone());
    }
    let _ = app.emit("learning-deep-link", target);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn accept_urls(app: &tauri::AppHandle, values: impl IntoIterator<Item = String>) {
    for value in values {
        match parse_learning_deep_link(&value) {
            Ok(target) => publish(app, target),
            Err(code) => log::warn!("Rejected learning deep link: {code}"),
        }
    }
}

pub fn accept_process_arguments(app: &tauri::AppHandle, values: impl IntoIterator<Item = String>) {
    accept_urls(
        app,
        values
            .into_iter()
            .filter(|value| value.starts_with("lynx://")),
    );
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_deep_link::DeepLinkExt;

    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    if !crate::isolated_test_instance() {
        app.deep_link().register_all()?;
    }

    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        accept_urls(&handle, event.urls().iter().map(ToString::to_string));
    });
    if let Some(urls) = app.deep_link().get_current()? {
        accept_urls(app.handle(), urls.into_iter().map(|url| url.to_string()));
    }
    Ok(())
}

#[tauri::command]
pub fn learning_take_pending_deep_link(
    state: tauri::State<'_, LearningDeepLinkState>,
) -> Option<LearningDeepLinkTarget> {
    state.pending.lock().ok()?.take()
}

#[cfg(test)]
mod tests {
    use super::{parse_learning_deep_link, LearningDeepLinkTarget};

    #[test]
    fn pasted_official_course_links_use_the_existing_target() {
        for origin in ["https://100ask.net", "https://www.100ask.net"] {
            for suffix in ["", "/", "/chapters/ch-first"] {
                let input = format!(" {origin}/lynx-learn/class-linux-app{suffix} ");
                let actual = super::learn_parse_course_link(input).unwrap();
                let expected = parse_learning_deep_link(&format!(
                    "lynx://learn/class-linux-app{}",
                    suffix.trim_end_matches('/')
                ))
                .unwrap();
                assert_eq!(actual, expected);
            }
        }
        assert!(super::learn_parse_course_link("lynx://learn/preview/test".into()).is_ok());
    }

    #[test]
    fn pasted_links_reject_other_sites_credentials_and_ambiguous_paths() {
        for input in [
            "http://100ask.net/lynx-learn/class-test",
            "https://evil.example/lynx-learn/class-test",
            "https://100ask.net.evil.example/lynx-learn/class-test",
            "https://name:secret@100ask.net/lynx-learn/class-test",
            "https://100ask.net:444/lynx-learn/class-test",
            "https://100ask.net/course/139",
            "https://100ask.net/admin/lynx-learn/class-test",
            "https://100ask.net/lynx-learn/class-test?token=secret",
            "https://100ask.net/lynx-learn/class-test#run",
            "https://100ask.net/lynx-learn/class-test/../class-other",
            "https://100ask.net/lynx-learn/class-test/chapters/%2e%2e",
            "https://100ask.net/lynx-learn/class-test/chapters/x/run",
            "https://100ask.net/lynx-learn/class-test//",
        ] {
            assert!(super::learn_parse_course_link(input.into()).is_err());
        }
        assert!(super::learn_parse_course_link("x".repeat(2049)).is_err());
    }

    #[test]
    fn framework_routes_target_chapters_without_executing_actions() {
        let target =
            parse_learning_deep_link("lynx://learn/class-linux-app/chapters/ch-first").unwrap();
        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({"courseSlug":"class-linux-app", "chapterId":"ch-first"})
        );
        assert!(parse_learning_deep_link("lynx://learn/class-bsp").is_ok());
        for bad in [
            "lynx://learn/class-/chapters/x",
            "lynx://learn/class-bsp/chapters/..",
            "lynx://learn/class-bsp/chapters/x/run",
            "lynx://learn/class-bsp?token=secret",
        ] {
            assert!(parse_learning_deep_link(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn preview_is_a_separate_target_and_legacy_json_is_preserved() {
        let preview = parse_learning_deep_link("lynx://learn/preview/preview_kvm_123")
            .expect("website preview link");
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            serde_json::json!({
                "previewId": "preview_kvm_123"
            })
        );
        let legacy = parse_learning_deep_link("lynx://learn/course/kvm").unwrap();
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({
                "courseSlug": "kvm", "labKey": null
            })
        );
    }

    #[test]
    fn unsafe_preview_routes_are_rejected() {
        for value in [
            "lynx://learn/preview/",
            "lynx://learn/preview/preview%2Fkvm",
            "lynx://learn/preview/preview_kvm/run",
            "lynx://learn/preview/preview_kvm?token=secret",
            "lynx://learn/preview/preview_kvm#run",
            "lynx://user:secret@learn/preview/preview_kvm",
        ] {
            assert!(parse_learning_deep_link(value).is_err(), "accepted {value}");
        }
        assert!(
            parse_learning_deep_link(&format!("lynx://learn/preview/{}", "x".repeat(129))).is_err()
        );
    }

    #[test]
    fn exact_course_and_lab_route_is_accepted() {
        let target =
            parse_learning_deep_link("lynx://learn/course/linux-driver/lab/char-device-01")
                .expect("official learning link");
        assert_eq!(
            target,
            LearningDeepLinkTarget::Course {
                course_slug: "linux-driver".into(),
                lab_key: Some("char-device-01".into())
            }
        );
    }

    #[test]
    fn course_only_route_is_accepted() {
        let target = parse_learning_deep_link("lynx://learn/course/linux-driver")
            .expect("official course link");
        assert_eq!(
            target,
            LearningDeepLinkTarget::Course {
                course_slug: "linux-driver".into(),
                lab_key: None
            }
        );
    }

    #[test]
    fn credentials_queries_fragments_and_unknown_actions_are_rejected() {
        for value in [
            "lynx://user:secret@learn/course/linux-driver",
            "lynx://learn/course/linux-driver?token=secret",
            "lynx://learn/course/linux-driver#run",
            "lynx://learn/course/linux-driver/lab/char-device-01/run",
            "lynx://kit/imx6ull/setup",
            "https://learn/course/linux-driver",
        ] {
            assert!(parse_learning_deep_link(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn encoded_or_unbounded_identifiers_are_rejected() {
        assert!(parse_learning_deep_link("lynx://learn/course/linux%2Fdriver").is_err());
        let oversized = "x".repeat(129);
        assert!(parse_learning_deep_link(&format!("lynx://learn/course/{oversized}")).is_err());
    }
}

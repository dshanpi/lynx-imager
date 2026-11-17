//! Invitation locators only: every list/open rechecks online account and signatures.
use super::*;
use crate::learning::local::LocalLearningState;
use futures_util::{stream, StreamExt};
use rusqlite::{params, Connection};

pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_preview_locations(base TEXT NOT NULL,user_id INTEGER NOT NULL,preview_id TEXT NOT NULL,digest TEXT NOT NULL,unit_id TEXT NOT NULL DEFAULT '',PRIMARY KEY(base,user_id,preview_id));")
}
fn storage_error() -> LearnError {
    error("LEARN_PREVIEW_STORAGE", "试学入口保存失败，请重试", true)
}
fn remember(db: &Connection, base: &str, p: &VerifiedPreview) -> Result<String, LearnError> {
    db.execute("INSERT INTO learn_preview_locations(base,user_id,preview_id,digest) VALUES(?1,?2,?3,?4) ON CONFLICT(base,user_id,preview_id) DO UPDATE SET unit_id=CASE WHEN digest=excluded.digest THEN unit_id ELSE '' END,digest=excluded.digest", params![base,p.binding.user_id,p.binding.preview_id,p.binding.content_digest]).map_err(|_|storage_error())?;
    db.query_row("SELECT unit_id FROM learn_preview_locations WHERE base=?1 AND user_id=?2 AND preview_id=?3",params![base,p.binding.user_id,p.binding.preview_id],|r|r.get(0)).map_err(|_|storage_error())
}
fn remembered(db: &Connection, base: &str, user: i64) -> Result<Vec<String>, LearnError> {
    let mut stmt=db.prepare("SELECT preview_id FROM learn_preview_locations WHERE base=?1 AND user_id=?2 ORDER BY rowid DESC LIMIT 129").map_err(|_|storage_error())?;
    let ids = stmt
        .query_map(params![base, user], |r| r.get(0))
        .map_err(|_| storage_error())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| storage_error())?;
    if ids.len() > 128 {
        return Err(schema_error());
    }
    Ok(ids)
}
pub(super) fn save_open(
    local: &LocalLearningState,
    base: &str,
    p: &mut VerifiedPreview,
) -> Result<(), LearnError> {
    let db = local.db.lock().map_err(|_| storage_error())?;
    let saved = remember(&db, base, p)?;
    p.current_unit_id = p.binding.allowed_unit_ids.contains(&saved).then_some(saved);
    Ok(())
}
pub(super) fn save_unit(
    local: &LocalLearningState,
    base: &str,
    p: &VerifiedPreview,
    unit: &str,
) -> Result<(), LearnError> {
    let db = local.db.lock().map_err(|_| storage_error())?;
    remember(&db, base, p)?;
    db.execute("UPDATE learn_preview_locations SET unit_id=?1 WHERE base=?2 AND user_id=?3 AND preview_id=?4 AND digest=?5",params![unit,base,p.binding.user_id,p.binding.preview_id,p.binding.content_digest]).map_err(|_|storage_error())?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Invitation {
    preview_id: String,
}
#[derive(Deserialize)]
struct InvitationData {
    previews: Vec<Invitation>,
}
fn parse_list(raw: &[u8]) -> Result<Vec<String>, LearnError> {
    #[derive(Deserialize)]
    struct Response {
        data: InvitationData,
    }
    let r: Response = serde_json::from_slice(raw).map_err(|_| schema_error())?;
    let ids: Vec<_> = r.data.previews.into_iter().map(|p| p.preview_id).collect();
    let unique: HashSet<_> = ids.iter().collect();
    if ids.len() > 128 || unique.len() != ids.len() || ids.iter().any(|id| !manifest::valid_id(id))
    {
        return Err(schema_error());
    }
    Ok(ids)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSummary {
    preview_id: String,
    title: String,
    expires_at: DateTime<Utc>,
    draft_revision: i64,
    chapter_count: usize,
    unit_count: usize,
    current_unit_id: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewList {
    items: Vec<PreviewSummary>,
    discovery_available: bool,
    warnings: Vec<String>,
}

#[tauri::command]
pub async fn learn_list_previews(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
) -> Result<PreviewList, LearnError> {
    let (base, token, user) = context(&auth).await?;
    let mut ids = {
        let db = local.db.lock().map_err(|_| storage_error())?;
        remembered(&db, &base, user)?
    };
    let mut result = PreviewList {
        items: vec![],
        discovery_available: false,
        warnings: vec![],
    };
    match super::super::get_response(courses::url(&base, &["previews"])?, &token, 65536, true).await
    {
        Ok(Some(raw)) => {
            let remote = parse_list(&raw)?;
            result.discovery_available = true;
            for id in remote {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Ok(None) => {}
        Err(e) if e.retryable => result
            .warnings
            .push("官网试学清单暂时不可用，正在核对已接收的邀请。".into()),
        Err(e) => return Err(e),
    }
    if ids.len() > 128 {
        return Err(schema_error());
    }
    let keys = manifest::official_keys()?;
    let checks = stream::iter(ids.clone())
        .map(|id| {
            let base = &base;
            let token = &token;
            let keys = &keys;
            async move { preview(base, token, &id, user, keys).await }
        })
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
    for (id, checked) in ids.iter().zip(checks) {
        match checked {
            Ok(mut p) => {
                save_open(&local, &base, &mut p)?;
                result.items.push(PreviewSummary {
                    preview_id: p.binding.preview_id,
                    title: p.course.manifest.title,
                    expires_at: p.binding.expires_at,
                    draft_revision: p.binding.draft_revision,
                    chapter_count: p.course.manifest.chapters.len(),
                    unit_count: p.binding.allowed_unit_ids.len(),
                    current_unit_id: p.current_unit_id,
                });
            }
            Err(e) => {
                // Never delete on a transient failure. Reopening always repeats authorization.
                if matches!(
                    e.machine_code,
                    "LEARN_PREVIEW_EXPIRED" | "LEARN_NOT_FOUND" | "LEARN_VALIDATION_STALE"
                ) {
                    let db = local.db.lock().map_err(|_| storage_error())?;
                    db.execute("DELETE FROM learn_preview_locations WHERE base=?1 AND user_id=?2 AND preview_id=?3",params![base,user,id]).map_err(|_|storage_error())?;
                }
                result
                    .warnings
                    .push(format!("一个试学邀请未能通过核验：{}", e.message));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locators_are_scoped_and_revision_change_resets_position() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let (raw, keys) = super::super::tests::preview_fixture();
        let mut p = verify_preview(raw, "preview_test", 7, &keys).unwrap();
        assert_eq!(remember(&db, "https://www.100ask.net", &p).unwrap(), "");
        db.execute(
            "UPDATE learn_preview_locations SET unit_id='unit_hello'",
            [],
        )
        .unwrap();
        assert_eq!(
            remember(&db, "https://www.100ask.net", &p).unwrap(),
            "unit_hello"
        );
        assert!(remembered(&db, "https://www.100ask.net", 8)
            .unwrap()
            .is_empty());
        assert!(remembered(&db, "https://other.example", 7)
            .unwrap()
            .is_empty());
        p.binding.content_digest = "b".repeat(64);
        assert_eq!(remember(&db, "https://www.100ask.net", &p).unwrap(), "");
    }
    #[test]
    fn discovery_requires_bounded_unique_valid_identifiers() {
        assert_eq!(
            parse_list(br#"{"data":{"previews":[{"previewId":"preview_test"}]}}"#).unwrap(),
            vec!["preview_test"]
        );
        for raw in [
            r#"{"data":[]}"#,
            r#"{"data":{"previews":[{"previewId":"../escape"}]}}"#,
            r#"{"data":{"previews":[{"previewId":"a"},{"previewId":"a"}]}}"#,
        ] {
            assert!(parse_list(raw.as_bytes()).is_err());
        }
    }
}

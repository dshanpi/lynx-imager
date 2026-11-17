//! Learn V2 projections. Read-only navigation never advances the learning state machine.
use super::*;

pub(super) fn migrate_journey(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learning_drafts(session_id TEXT NOT NULL,step_id TEXT NOT NULL,attempt INTEGER NOT NULL,answer TEXT NOT NULL,reflection TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(session_id,step_id,attempt));
        CREATE TABLE IF NOT EXISTS learning_archives(session_id TEXT PRIMARY KEY,payload TEXT NOT NULL,updated_at INTEGER NOT NULL);")
}

fn clean_text(value: &str) -> String {
    crate::supervisor::redact_learning_context(&value.chars().take(4000).collect::<String>())
}

/// Enumerated navigation suggestions, not executable tool requests.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LearnAgentDestination {
    Text,
    Engineering,
    Evidence,
    Assessment,
    Records,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnAgentPlan {
    session_id: String,
    step_id: String,
    attempt: i64,
    state: String,
    title: String,
    guidance: String,
    destination: LearnAgentDestination,
    requires_user_action: bool,
    executed_action: bool,
}

fn agent_next(state: &str) -> Result<(LearnAgentDestination, &'static str, &'static str), String> {
    use LearnAgentDestination::*;
    Ok(match state {
        "reading" => (
            Text,
            "理解本课目标",
            "先阅读目标、预期现象与教材，再用自己的话说明要解决的问题。",
        ),
        "checking_understanding" => (
            Text,
            "完成理解检查",
            "保存你的判断；不清楚时先请求一级提示，再核对教材。",
        ),
        "practicing" => (
            Engineering,
            "核对工程并开始实践",
            "确认 Ubuntu 工程、源码版本和板卡目标；在 VS Code 修改后返回实践面板执行课程动作。",
        ),
        "collecting_evidence" => (
            Evidence,
            "收集本次实践证据",
            "保留当前尝试的命令结果和真实现象；未验证的设备状态不能当作通过。",
        ),
        "verifying" => (
            Evidence,
            "检查规则与证据缺口",
            "按课程验收规则核对结果；失败后先分析原因，重试需要你重新发起。",
        ),
        "reflecting" => (
            Assessment,
            "完成章节复盘",
            "解释现象和排查过程，记录仍未理解的问题，然后提交本课反思。",
        ),
        "completed" => (
            Records,
            "查看课程学习档案",
            "回顾问题、证据与每次尝试，导出学习记录并制定下一阶段计划。",
        ),
        _ => return Err("LEARN_AGENT_STATE_INVALID".into()),
    })
}

#[command]
pub fn learning_get_agent_plan(
    session_id: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LearnAgentPlan, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &session_id)?;
    let (destination, title, guidance) = agent_next(&session.state)?;
    Ok(LearnAgentPlan {
        attempt: current_attempt(&db, &session_id, &session.current_step)?,
        session_id,
        step_id: session.current_step,
        state: session.state,
        title: title.into(),
        guidance: guidance.into(),
        destination,
        requires_user_action: true,
        executed_action: false,
    })
}

pub(super) fn ensure_agent_context(
    db: &Connection,
    original: &LocalLearningSession,
    attempt: i64,
) -> Result<(), String> {
    let current = get_session(db, &original.session_id)?;
    if current.current_step != original.current_step
        || current.state != original.state
        || current_attempt(db, &original.session_id, &original.current_step)? != attempt
    {
        return Err("LEARN_AGENT_CONTEXT_CHANGED".into());
    }
    Ok(())
}

pub(super) fn agent_context(
    db: &Connection,
    session: &LocalLearningSession,
) -> Result<serde_json::Value, String> {
    let attempt = current_attempt(db, &session.session_id, &session.current_step)?;
    let draft: (String,String) = db.query_row("SELECT answer,reflection FROM learning_drafts WHERE session_id=?1 AND step_id=?2 AND attempt=?3",params![session.session_id,session.current_step,attempt],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?.unwrap_or_default();
    let mut stmt = db.prepare("SELECT kind,source,status,summary FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND kind != 'tutor' ORDER BY id DESC LIMIT 3").map_err(|e|e.to_string())?;
    let evidence=stmt.query_map(params![session.session_id,session.current_step,attempt], |r|{
        let summary:String=r.get(3)?;
        Ok(serde_json::json!({"kind":r.get::<_,String>(0)?,"source":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"summary":clean_text(&summary.chars().take(800).collect::<String>())}))
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    Ok(
        serde_json::json!({"courseVersion":session.course_version,"state":session.state,"attempt":attempt,"answer":clean_text(&draft.0.chars().take(1000).collect::<String>()),"reflection":clean_text(&draft.1.chars().take(1000).collect::<String>()),"evidence":evidence,"allowsExecution":false}),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnAttempt {
    step_id: String,
    attempt: i64,
    state: String,
    answer: String,
    reflection: String,
    updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnJourney {
    activity_access: Vec<prerequisites::Access>,
    title: String,
    labs: Vec<LocalLab>,
    attempts: Vec<LearnAttempt>,
    attempt: i64,
    answer: String,
    reflection: String,
    evidence: Vec<LearnEvidence>,
    notes: Vec<LearnTutorNote>,
    saved_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnTutorNote {
    step_id: String,
    attempt: i64,
    question: String,
    answer: String,
    source: String,
    created_at: i64,
}

pub(super) fn tutor_detail(question: &str, answer: &str, hint_level: &str) -> serde_json::Value {
    serde_json::json!({ "hintLevel": hint_level, "questionSha256": sha256(question.as_bytes()), "answerSha256": sha256(answer.as_bytes()), "question": clean_text(question), "answer": clean_text(answer) })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnDocument {
    markdown: String,
    source_url: String,
    sha256: String,
}

fn read_document(root: &Path, document: &LearnDocumentSpec) -> Result<LearnDocument, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "LEARN_DOCUMENT_ROOT_MISSING")?;
    let path = root
        .join(safe_relative(&document.path)?)
        .canonicalize()
        .map_err(|_| "LEARN_DOCUMENT_NOT_FOUND")?;
    if !path.starts_with(&root) {
        return Err("LEARN_DOCUMENT_PATH_ESCAPE".into());
    }
    let mut file = fs::File::open(path).map_err(|_| "LEARN_DOCUMENT_NOT_FOUND")?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "LEARN_DOCUMENT_READ_FAILED")?;
    if bytes.len() > 512 * 1024 {
        return Err("LEARN_DOCUMENT_TOO_LARGE".into());
    }
    if sha256(&bytes) != document.sha256 {
        return Err("LEARN_DOCUMENT_HASH_MISMATCH".into());
    }
    Ok(LearnDocument {
        markdown: String::from_utf8(bytes).map_err(|_| "LEARN_DOCUMENT_ENCODING")?,
        source_url: document.source_url.clone(),
        sha256: document.sha256.clone(),
    })
}

#[command]
pub fn learning_read_document(
    session_id: String,
    step_id: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<Option<LearnDocument>, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &session_id)?;
    let (manifest, _) = manifest_and_lab(&db, &session)?;
    let lab = manifest
        .labs
        .iter()
        .find(|lab| lab.step_id == step_id)
        .ok_or("LEARNING_STEP_NOT_FOUND")?;
    let Some(document) = &lab.document else {
        return Ok(None);
    };
    let root: String = db
        .query_row(
            "SELECT install_path FROM local_courses WHERE course_id=?1 AND version=?2",
            params![session.course_id, session.course_version],
            |r| r.get(0),
        )
        .map_err(|_| "LOCAL_COURSE_NOT_INSTALLED")?;
    read_document(Path::new(&root), document).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnEvidence {
    recorded: Option<hil_trust::RecordedEvidence>,
    step_id: String,
    attempt: i64,
    kind: String,
    source: String,
    status: String,
    summary: String,
    digest: String,
    created_at: i64,
}

fn read_journey(db: &Connection, session_id: &str) -> Result<LearnJourney, String> {
    let session = get_session(db, session_id)?;
    let (manifest, _) = manifest_and_lab(db, &session)?;
    let attempt = current_attempt(db, session_id, &session.current_step)?;
    let mut stmt = db.prepare("SELECT step_id,attempt,state,answer,reflection,updated_at FROM learning_attempts WHERE session_id=?1 ORDER BY id").map_err(|e| e.to_string())?;
    let attempts = stmt
        .query_map([session_id], |r| {
            Ok(LearnAttempt {
                step_id: r.get(0)?,
                attempt: r.get(1)?,
                state: r.get(2)?,
                answer: clean_text(&r.get::<_, String>(3)?),
                reflection: clean_text(&r.get::<_, String>(4)?),
                updated_at: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let draft: Option<(String, String)> = db.query_row("SELECT answer,reflection FROM learning_drafts WHERE session_id=?1 AND step_id=?2 AND attempt=?3", params![session_id, session.current_step, attempt], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|e| e.to_string())?;
    let current = attempts
        .iter()
        .find(|a| a.step_id == session.current_step && a.attempt == attempt);
    let (answer, reflection) = draft.unwrap_or_else(|| {
        current
            .map(|a| (a.answer.clone(), a.reflection.clone()))
            .unwrap_or_default()
    });
    let mut stmt = db.prepare("SELECT step_id,attempt,kind,source,status,summary,digest,created_at FROM learning_evidence WHERE session_id=?1 ORDER BY id DESC LIMIT 1000").map_err(|e| e.to_string())?;
    let evidence = stmt
        .query_map([session_id], |r| {
            let mut value = LearnEvidence {
                recorded: None,
                step_id: r.get(0)?,
                attempt: r.get(1)?,
                kind: r.get(2)?,
                source: r.get(3)?,
                status: r.get(4)?,
                summary: clean_text(&r.get::<_, String>(5)?),
                digest: r.get(6)?,
                created_at: r.get(7)?,
            };
            value.recorded = hil_trust::project_evidence(
                &mut value.source,
                &mut value.status,
                &mut value.summary,
            );
            Ok(value)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT step_id,attempt,detail_json,source,created_at FROM learning_evidence WHERE session_id=?1 AND kind='tutor' ORDER BY id DESC LIMIT 200").map_err(|e| e.to_string())?;
    let notes = stmt
        .query_map([session_id], |r| {
            let detail: String = r.get(2)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null);
            Ok(LearnTutorNote {
                step_id: r.get(0)?,
                attempt: r.get(1)?,
                question: clean_text(parsed["question"].as_str().unwrap_or_default()),
                answer: clean_text(parsed["answer"].as_str().unwrap_or_default()),
                source: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|note| !note.question.is_empty())
        .collect();
    let activity_access = prerequisites::evaluate(db, &session, &manifest)?;
    Ok(LearnJourney {
        activity_access,
        title: manifest.title,
        labs: manifest.labs,
        attempts,
        attempt,
        answer: clean_text(&answer),
        reflection: clean_text(&reflection),
        evidence,
        notes,
        saved_at: now(),
    })
}

#[command]
pub fn learning_get_journey(
    session_id: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<LearnJourney, String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let journey = read_journey(&db, &session_id)?;
    // Archive only scoped facts. No credentials, raw command output or order numbers.
    let payload = serde_json::to_string(&journey).map_err(|e| e.to_string())?;
    db.execute("INSERT INTO learning_archives VALUES(?1,?2,?3) ON CONFLICT(session_id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at", params![session_id, payload, journey.saved_at]).map_err(|e| e.to_string())?;
    Ok(journey)
}

fn save_draft(
    db: &Connection,
    session_id: &str,
    step_id: &str,
    attempt: i64,
    answer: &str,
    reflection: &str,
) -> Result<(), String> {
    let session = get_session(db, session_id)?;
    if session.current_step != step_id || current_attempt(db, session_id, step_id)? != attempt {
        return Err("LEARN_DRAFT_CONTEXT_CHANGED".into());
    }
    if answer.chars().count() > 4000 || reflection.chars().count() > 4000 {
        return Err("LEARNING_TEXT_TOO_LARGE".into());
    }
    db.execute("INSERT INTO learning_drafts VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,step_id,attempt) DO UPDATE SET answer=excluded.answer,reflection=excluded.reflection,updated_at=excluded.updated_at", params![session_id, step_id, attempt, clean_text(answer), clean_text(reflection), now()]).map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
pub fn learning_save_draft(
    session_id: String,
    step_id: String,
    attempt: i64,
    answer: String,
    reflection: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    save_draft(&db, &session_id, &step_id, attempt, &answer, &reflection)
}

fn decode_photo(value: &str) -> Result<Vec<u8>, String> {
    if value.len() > 8 * 1024 * 1024 {
        return Err("LEARN_PHOTO_TOO_LARGE".into());
    }
    let bytes = STANDARD.decode(value).map_err(|_| "LEARN_PHOTO_INVALID")?;
    if bytes.len() < 33 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return Err("LEARN_PHOTO_INVALID".into());
    }
    let width = u32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| "LEARN_PHOTO_INVALID")?,
    );
    let height = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| "LEARN_PHOTO_INVALID")?,
    );
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("LEARN_PHOTO_DIMENSIONS".into());
    }
    Ok(bytes)
}

#[command]
pub fn learning_attach_photo(
    session_id: String,
    step_id: String,
    attempt: i64,
    png_base64: String,
    state: State<'_, Arc<LocalLearningState>>,
) -> Result<String, String> {
    let bytes = decode_photo(&png_base64)?;
    let db = state.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    let session = get_session(&db, &session_id)?;
    if session.current_step != step_id || current_attempt(&db, &session_id, &step_id)? != attempt {
        return Err("LEARN_PHOTO_CONTEXT_CHANGED".into());
    }
    let digest = sha256(&bytes);
    let directory = state.root.join("learning/observations");
    fs::create_dir_all(&directory).map_err(|_| "LEARN_PHOTO_SAVE_FAILED")?;
    let path = directory.join(format!("{digest}.png"));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(&bytes)
            .map_err(|_| "LEARN_PHOTO_SAVE_FAILED")?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if sha256(&fs::read(&path).map_err(|_| "LEARN_PHOTO_READ_FAILED")?) != digest {
                return Err("LEARN_PHOTO_DIGEST_MISMATCH".into());
            }
        }
        Err(_) => return Err("LEARN_PHOTO_SAVE_FAILED".into()),
    }
    append_evidence(
        &db,
        &session,
        "photo",
        "user-camera",
        "RECORDED",
        "用户单次拍摄；未进行视觉分析；不代表板卡验证通过",
        &serde_json::json!({ "mediaSha256": digest, "bytes": bytes.len(), "boardUid": session.board_uid, "scope": "single-frame", "sentToAi": false }),
    )?;
    Ok(digest)
}

fn bound_project(
    local: &LocalLearningState,
    workbench: &WorkbenchState,
    session_id: &str,
) -> Result<
    (
        crate::workbench::types::ProjectConfig,
        crate::workbench::types::HostConfig,
    ),
    String,
> {
    let session = {
        let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
        get_session(&db, session_id)?
    };
    let db = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    let project = crate::workbench::storage::project_by_id(&db, session.project_id)
        .map_err(|_| "LEARNING_PROJECT_NOT_FOUND")?;
    let host = crate::workbench::storage::host_by_id(&db, project.host_id)
        .map_err(|_| "LEARNING_HOST_NOT_FOUND")?;
    Ok((project, host))
}

#[cfg(test)]
fn ide_uri(username: &str, address: &str, port: u16, path: &str) -> Result<String, String> {
    let host = serde_json::from_value(serde_json::json!({"id":1,"name":"test","address":address,"port":port,"username":username,"authType":"password","privateKeyPath":"","hostFingerprint":"SHA256:test","defaultWorkDir":"/tmp"})).map_err(|_| "LEARN_IDE_HOST_INVALID")?;
    crate::learning::ide::ide_url(&host, path, "", "")
}

#[command]
pub fn learning_open_ide(
    session_id: String,
    ssh_alias: Option<String>,
    relative_file: Option<String>,
    app: tauri::AppHandle,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let (project, host) = bound_project(&local, &workbench, &session_id)?;
    let uri = crate::learning::ide::checked_url(
        &host,
        &project.sdk_dir,
        ssh_alias.as_deref().unwrap_or_default(),
        relative_file.as_deref().unwrap_or_default(),
    )?;
    app.opener()
        .open_url(uri, None::<String>)
        .map_err(|_| "LEARN_IDE_OPEN_FAILED:请安装 VS Code 与 Remote SSH 扩展".into())
}

#[command]
pub async fn learning_inspect_project(
    session_id: String,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
) -> Result<super::environment::Snapshot, String> {
    super::environment::inspect(&local, &workbench, &session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_navigation_is_enumerated_and_never_accepts_an_execution_state() {
        assert_eq!(
            agent_next("practicing").unwrap().0,
            LearnAgentDestination::Engineering
        );
        assert_eq!(
            agent_next("verifying").unwrap().0,
            LearnAgentDestination::Evidence
        );
        assert_eq!(
            agent_next("completed").unwrap().0,
            LearnAgentDestination::Records
        );
        assert!(agent_next("run_shell").is_err());
    }

    #[test]
    fn agent_context_excludes_other_attempts_and_rejects_stale_responses() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('s','c','1',1,'one','reading',1,1)",[]).unwrap();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('s','one',1,'reading',1,1)",[]).unwrap();
        save_draft(
            &db,
            "s",
            "one",
            1,
            "password=secret-demo-value",
            "reflection",
        )
        .unwrap();
        let original = get_session(&db, "s").unwrap();
        for _ in 0..5 {
            append_evidence(
                &db,
                &original,
                "command",
                "software",
                "FAIL",
                "password=secret-demo-value",
                &serde_json::json!({}),
            )
            .unwrap();
        }
        let context = agent_context(&db, &original).unwrap();
        assert!(!context.to_string().contains("secret-demo-value"));
        assert_eq!(context["evidence"].as_array().unwrap().len(), 3);
        assert_eq!(context["allowsExecution"], false);
        ensure_agent_context(&db, &original, 1).unwrap();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('s','one',2,'reading',1,1)",[]).unwrap();
        assert_eq!(
            ensure_agent_context(&db, &original, 1).unwrap_err(),
            "LEARN_AGENT_CONTEXT_CHANGED"
        );
        assert!(agent_context(&db, &original).unwrap()["evidence"]
            .as_array()
            .unwrap()
            .is_empty());
        db.execute(
            "UPDATE learning_sessions SET state='practicing' WHERE session_id='s'",
            [],
        )
        .unwrap();
        assert!(ensure_agent_context(&db, &original, 2).is_err());
    }

    #[test]
    fn tutor_notes_redact_before_persistence() {
        let detail = tutor_detail("password=secret-demo-value", "Review the board logs", "L1");
        assert!(!detail.to_string().contains("secret-demo-value"));
        assert_eq!(detail["answer"], "Review the board logs");
        assert_eq!(detail["hintLevel"], "L1");
    }

    #[test]
    fn document_requires_fixed_digest_and_cannot_escape_root() {
        let root = std::env::temp_dir().join(format!("lynx-learn-doc-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("lesson.md"), "# lesson").unwrap();
        let mut document = LearnDocumentSpec {
            path: "lesson.md".into(),
            sha256: sha256(b"# lesson"),
            source_url: "https://example.org/lesson".into(),
        };
        assert_eq!(
            read_document(&root, &document).unwrap().markdown,
            "# lesson"
        );
        fs::write(root.join("lesson.md"), "changed").unwrap();
        assert!(read_document(&root, &document).is_err());
        document.path = "../outside.md".into();
        assert!(read_document(&root, &document).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn photos_reject_non_png_and_excessive_payloads() {
        assert!(decode_photo(&STANDARD.encode(b"not an image")).is_err());
        assert!(decode_photo(&"a".repeat(8 * 1024 * 1024 + 1)).is_err());
    }

    #[test]
    fn local_ide_literal_percent_must_not_be_decoded_as_traversal() {
        assert_eq!(
            ide_uri("student", "host", 22, "/sdk/%2e%2e/private").unwrap(),
            "vscode://vscode-remote/ssh-remote+student@host/sdk/%252e%252e/private"
        );
    }

    #[test]
    fn ide_uri_keeps_path_data_and_rejects_ambiguous_hosts() {
        let uri = ide_uri("student", "192.168.1.20", 22, "/home/student/My SDK").unwrap();
        assert!(uri.starts_with("vscode://vscode-remote/ssh-remote+student@192.168.1.20/"));
        assert!(uri.contains("My%20SDK"));
        assert!(ide_uri("a", "host/other", 22, "/sdk").is_err());
        assert!(ide_uri("a", "host", 2222, "/sdk").is_err());
        assert!(ide_uri("a", "host", 22, "relative").is_err());
    }

    #[test]
    fn drafts_are_attempt_scoped_and_redacted() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute("INSERT INTO learning_sessions(session_id,course_id,course_version,project_id,current_step,state,created_at,updated_at) VALUES('s','c','1',1,'one','reading',1,1)", []).unwrap();
        db.execute("INSERT INTO learning_attempts(session_id,step_id,attempt,state,created_at,updated_at) VALUES('s','one',1,'reading',1,1)", []).unwrap();
        save_draft(&db, "s", "one", 1, "my answer", "reflection").unwrap();
        assert!(save_draft(&db, "s", "old", 1, "bad", "").is_err());
        assert!(save_draft(&db, "s", "one", 2, "bad", "").is_err());
        let answer: String = db
            .query_row("SELECT answer FROM learning_drafts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(answer, "my answer");
        assert!(!clean_text("password=secret-demo-value").contains("secret-demo-value"));
    }
}

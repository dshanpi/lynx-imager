//! Explicit, scoped local diagnostics. Never exposes arbitrary evidence JSON.
use super::*;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticRequest {
    pub session_id: String,
    pub step_id: String,
    pub attempt: i64,
    pub digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCommandDiagnostic {
    #[serde(flatten)]
    scope: DiagnosticScope,
    pub(super) status: String,
    pub(super) availability: String,
    pub(super) text: Option<String>,
    pub(super) truncated: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) expected_exit_code: Option<i32>,
    output_sha256: Option<String>,
    created_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticScope {
    session_id: String,
    step_id: String,
    attempt: i64,
    digest: String,
}

const INVALID: &str = "LEARNING_DIAGNOSTIC_INVALID:诊断记录未通过完整性检查";
const MISSING: &str = "LEARNING_DIAGNOSTIC_NOT_FOUND:未找到此尝试的命令诊断";

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(super) fn read(
    db: &Connection,
    request: &DiagnosticRequest,
) -> Result<LocalCommandDiagnostic, String> {
    if request.session_id.is_empty()
        || request.session_id.len() > 256
        || request.step_id.is_empty()
        || request.step_id.len() > 256
        || request.attempt < 1
        || !is_digest(&request.digest)
    {
        return Err(INVALID.into());
    }
    get_session(db, &request.session_id).map_err(|_| MISSING)?;
    let (status, detail, created_at): (String, Option<String>, i64) = db.query_row(
        "SELECT status,CASE WHEN length(CAST(detail_json AS BLOB))<=32768 THEN detail_json ELSE NULL END,created_at FROM learning_evidence WHERE session_id=?1 AND step_id=?2 AND attempt=?3 AND digest=?4 AND kind='command' AND source='software' ORDER BY id DESC LIMIT 1",
        params![request.session_id,request.step_id,request.attempt,request.digest],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(|_| INVALID)?.ok_or(MISSING)?;
    let detail = detail.ok_or(INVALID)?;
    if sha256(detail.as_bytes()) != request.digest
        || !["PASS", "FAIL", "ERROR"].contains(&status.as_str())
    {
        return Err(INVALID.into());
    }
    let value: serde_json::Value = serde_json::from_str(&detail).map_err(|_| INVALID)?;
    let mut result = LocalCommandDiagnostic {
        scope: DiagnosticScope {
            session_id: request.session_id.clone(),
            step_id: request.step_id.clone(),
            attempt: request.attempt,
            digest: request.digest.clone(),
        },
        status,
        availability: "legacy_unavailable".into(),
        text: None,
        truncated: false,
        exit_code: None,
        expected_exit_code: None,
        output_sha256: None,
        created_at,
    };
    // Earlier records truncated before redaction. Do not expose their unverifiable tail.
    if value.get("diagnosticVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Ok(result);
    }
    let text = value
        .get(if result.status == "ERROR" {
            "error"
        } else {
            "outputTail"
        })
        .and_then(|v| v.as_str())
        .ok_or(INVALID)?;
    if text.chars().count() > 2048 {
        return Err(INVALID.into());
    }
    result.truncated = value
        .get("outputTruncated")
        .and_then(|v| v.as_bool())
        .ok_or(INVALID)?;
    if result.status != "ERROR" {
        let code = |name| -> Result<i32, String> {
            value
                .get(name)
                .and_then(|v| v.as_i64())
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| INVALID.into())
        };
        result.exit_code = Some(code("exitCode")?);
        result.expected_exit_code = Some(code("expectedExitCode")?);
        let digest = value
            .get("outputSha256")
            .and_then(|v| v.as_str())
            .filter(|v| is_digest(v))
            .ok_or(INVALID)?;
        result.output_sha256 = Some(digest.into());
        if (result.exit_code == result.expected_exit_code) != (result.status == "PASS") {
            return Err(INVALID.into());
        }
    }
    result.availability = "available".into();
    result.text = Some(crate::learning::v2::tutor::redact_task_log(text));
    Ok(result)
}

#[command]
pub fn learning_get_local_command_diagnostic(
    request: DiagnosticRequest,
    local: State<'_, Arc<LocalLearningState>>,
) -> Result<LocalCommandDiagnostic, String> {
    let db = local.db.lock().map_err(|_| "LOCAL_COURSE_DB_UNAVAILABLE")?;
    read(&db, &request)
}

pub(super) fn tail(value: &str) -> (String, bool) {
    let redacted = crate::learning::v2::tutor::redact_task_log(value);
    let truncated = redacted.chars().count() > 2048;
    let text = redacted
        .chars()
        .rev()
        .take(2048)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (text, truncated)
}

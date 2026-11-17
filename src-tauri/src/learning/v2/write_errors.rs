//! A write is definitively rejected only when the bounded protocol says so.
use super::{error, LearnError};
use serde::Deserialize;

const MAX_ERROR_BYTES: usize = 16 * 1024;

fn unconfirmed() -> LearnError {
    error(
        "LEARN_WRITE_UNCONFIRMED",
        "尚不能确认原提交的结果，请保留原请求并重试确认",
        true,
    )
}

#[derive(Deserialize)]
struct Envelope {
    error: Conflict,
}
#[derive(Deserialize)]
struct Conflict {
    code: String,
    retryable: bool,
}

fn classify(bytes: &[u8]) -> LearnError {
    let Ok(envelope) = serde_json::from_slice::<Envelope>(bytes) else {
        return unconfirmed();
    };
    // Never echo server messages/details. These may contain private request data.
    match (envelope.error.code.as_str(), envelope.error.retryable) {
        ("LEARN_OPERATION_IN_PROGRESS", true) => error(
            "LEARN_OPERATION_IN_PROGRESS",
            "原提交仍在处理，请稍后使用同一请求确认结果",
            true,
        ),
        ("LEARN_REVISION_CONFLICT" | "LEARN_PREVIEW_STALE", false) => error(
            "LEARN_PREVIEW_STALE",
            "内容或请求已变化，请保留输入并同步最新记录",
            false,
        ),
        ("IDEMPOTENCY_CONFLICT", false) => error(
            "LEARN_IDEMPOTENCY_CONFLICT",
            "原请求标识已关联其他内容，请保留恢复记录并核对，不要创建新请求覆盖",
            false,
        ),
        _ => unconfirmed(),
    }
}

pub(super) async fn conflict(mut response: reqwest::Response) -> LearnError {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !content_type.eq_ignore_ascii_case("application/json")
        || response
            .content_length()
            .is_some_and(|size| size > MAX_ERROR_BYTES as u64)
    {
        return unconfirmed();
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= MAX_ERROR_BYTES => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => return classify(&bytes),
            _ => return unconfirmed(),
        }
    }
}

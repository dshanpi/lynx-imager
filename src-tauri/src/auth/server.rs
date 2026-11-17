//! OAuth2 回调服务（axum）。
//!
//! 仅在登录流程进行期间监听 `127.0.0.1:<port>/auth/100ask/callback`，接收 100ask.net
//! 回跳的授权码。handler 校验 state（防 CSRF）→ 换 token → 取用户信息 → 存会话 →
//! emit `auth-login-result` 事件给前端 → 触发自身关停。
//!
//! 关停信号存在 [`AuthState`] 的运行句柄里（`Arc<Mutex<Option<Sender>>>`）。匹配回调
//! 与登录截止原子竞争 pending；处理中保留共享 claim，回调提交或用户取消再原子取得
//! 服务句柄。获胜方负责唯一终态与关停。

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Html,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::config::{exchange_code, fetch_userinfo, UserInfo};
use super::state::{AuthState, RunningAuthServer};
use crate::diagnostics::DiagnosticState;

/// `auth-login-result` 事件载荷。
///
/// 登录成功时 `success=true` 且带 `user`；失败时 `success=false` 且带 `error`。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoginResultEvent {
    pub success: bool,
    pub user: Option<UserInfo>,
    pub error: Option<String>,
}

/// 回调查询参数。
///
/// `code`/`state` 为正常授权回调；`error` 为用户拒绝授权等错误回调。
#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// 回调 handler 的共享上下文（廉价可 clone）。
#[derive(Clone)]
pub struct CallbackState {
    /// 登录状态单例（校验 state、存会话、关停回调服务；持有内置 OAuth 配置）。
    pub auth: Arc<AuthState>,
    /// Diagnostic state activated after the authenticated session is persisted.
    pub diagnostics: Arc<DiagnosticState>,
    /// 用于 emit `auth-login-result` 事件。
    pub app: AppHandle,
}

/// 构造回调路由。
pub fn router(cs: CallbackState) -> Router {
    Router::new()
        .route("/auth/100ask/callback", get(callback_handler))
        .with_state(cs)
}

/// 回调 handler：校验 → 换 token → 取用户信息 → 存会话 → emit → 关停。
async fn callback_handler(
    State(cs): State<CallbackState>,
    Query(q): Query<CallbackQuery>,
) -> Html<String> {
    let (result, server) = handle_callback(&cs, q).await;
    if let Some(server) = server {
        if !result.success {
            crate::diagnostics::capture_backend_error(
                &cs.diagnostics,
                "oauth_callback",
                "OAUTH_LOGIN_FAILED",
                "oauth_login",
            );
        }
        let _ = cs.app.emit("auth-login-result", result.clone());

        // 服务与 PKCE 一并归属本回调，不会误取后续流程的句柄。
        if let Some(tx) = server.shutdown.lock().ok().and_then(|mut g| g.take()) {
            let _ = tx.send(());
        }
        // join 由 axum graceful shutdown 自行结束，此处不 await（否则会阻塞响应回写）。
    }

    render_result_page(&result)
}

fn callback_failure(
    error: impl Into<String>,
    server: Option<RunningAuthServer>,
) -> (LoginResultEvent, Option<RunningAuthServer>) {
    (
        LoginResultEvent {
            success: false,
            user: None,
            error: Some(error.into()),
        },
        server,
    )
}

/// 实际处理回调。第二个返回值是本请求原子取得并负责关闭的服务。
async fn handle_callback(
    cs: &CallbackState,
    q: CallbackQuery,
) -> (LoginResultEvent, Option<RunningAuthServer>) {
    let pending_state = match q.state {
        Some(state) if !state.is_empty() => state,
        _ => {
            return callback_failure("回调缺少 state 参数", None);
        }
    };

    // 只有完整的 code 或显式拒绝才是终态候选。缺参请求不得消耗当前登录。
    if q.error.is_none() && !q.code.as_ref().is_some_and(|code| !code.is_empty()) {
        return callback_failure("回调缺少 code 参数", None);
    }

    // 原子取得 PKCE 并登记处理中；截止任务不能再取得本轮资源。
    let Some(pending) = cs.auth.claim_login_callback(&pending_state) else {
        return callback_failure("state 校验失败（可能登录已过期或存在 CSRF）", None);
    };

    if let Some(error) = q.error {
        let server = cs.auth.finish_login_callback(&pending_state);
        return callback_failure(format!("授权失败: {error}"), server);
    }

    let code = match q.code {
        Some(code) if !code.is_empty() => code,
        _ => {
            // 上方已检查终态形状；该分支只用于保持类型完整。
            let server = cs.auth.finish_login_callback(&pending_state);
            return callback_failure("回调缺少 code 参数", server);
        }
    };
    let code_verifier = pending.code_verifier;
    let Some(cancel) = cs.auth.callback_cancellation(&pending_state) else {
        return callback_failure("登录已取消或被新的登录替代", None);
    };

    // 换 token（PKCE：带 code_verifier，不传 client_secret）
    let token_result = tokio::select! {
        _ = cancel.cancelled() => {
            return callback_failure("登录已取消或被新的登录替代", None);
        }
        result = exchange_code(&cs.auth.http, &cs.auth.oauth, &code, &code_verifier) => result,
    };
    let token = match token_result {
        Ok(t) => t,
        Err(e) => {
            let server = cs.auth.finish_login_callback(&pending_state);
            return callback_failure(e, server);
        }
    };

    // 取用户信息
    let user_result = tokio::select! {
        _ = cancel.cancelled() => {
            return callback_failure("登录已取消或被新的登录替代", None);
        }
        result = fetch_userinfo(&cs.auth.http, &cs.auth.oauth, &token.access_token) => result,
    };
    let user = match user_result {
        Ok(u) => u,
        Err(e) => {
            let server = cs.auth.finish_login_callback(&pending_state);
            return callback_failure(e, server);
        }
    };

    // 取消/重试可在网络处理期间先取得服务。只有仍持有 claim 的回调可写会话。
    let Some(server) = cs.auth.finish_login_callback(&pending_state) else {
        return callback_failure("登录已取消或被新的登录替代", None);
    };

    // 存会话
    if let Err(error) = cs
        .auth
        .set_session(super::config::Session::with_refresh_token(
            user.clone(),
            token.access_token,
            token.expires_in,
            token.refresh_token,
        ))
    {
        return callback_failure(
            format!("登录成功，但安全保存会话失败: {error}"),
            Some(server),
        );
    }
    log::info!("100ask.net 登录成功: {} ({})", user.name, user.email);
    if let Err(error) = crate::diagnostics::activate_for_authenticated_session(
        cs.diagnostics.clone(),
        cs.auth.clone(),
        cs.app.clone(),
    ) {
        log::warn!("LYNX diagnostics could not be activated after login: {error}");
    }

    (
        LoginResultEvent {
            success: true,
            user: Some(user),
            error: None,
        },
        Some(server),
    )
}

/// 渲染回调结果页面（浏览器里给用户看的确认页）。
fn render_result_page(result: &LoginResultEvent) -> Html<String> {
    let body = if result.success {
        let u = result.user.as_ref();
        let name = u.map(|u| u.name.as_str()).unwrap_or("");
        let email = u.map(|u| u.email.as_str()).unwrap_or("");
        format!("<h2>✅ 登录成功</h2><p>{name}（{email}）</p><p>可以关闭此页面并回到应用。</p>")
    } else {
        format!(
            "<h2>❌ 登录失败</h2><p>{}</p>",
            result.error.as_deref().unwrap_or("未知错误")
        )
    };
    Html(format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>登录结果</title>\
         <style>body{{font-family:sans-serif;text-align:center;margin-top:60px;}}</style></head>\
         <body>{body}</body></html>"
    ))
}

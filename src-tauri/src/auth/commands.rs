//! 100ask.net OAuth2 登录的 Tauri 命令。
//!
//! - [`auth_login_start`]：生成 state → 起本机回调服务 → 返回授权地址给前端打开。命令本身立即返回，
//!   登录结果经 `auth-login-result` 事件异步回传前端（回调服务在收到回跳后自动关停）。
//! - [`auth_get_user`]：查询当前登录用户（未登录返回 null）。
//! - [`auth_logout`]：登出，清空会话。
//! - [`auth_cancel_login`]：取消进行中的登录，关停回调服务。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::config::{generate_pkce, random_string, UserInfo};
use super::server::{router, CallbackState, LoginResultEvent};
use super::state::{AuthState, RunningAuthServer};
use crate::diagnostics::DiagnosticState;

/// 登录超时（回调服务最长存活时间）。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// `auth_login_start` 返回值。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStartResult {
    /// 回调服务监听端口。
    pub port: u16,
    /// 回调地址（与 100ask.net 注册一致）。
    pub redirect_uri: String,
    /// 授权地址（已带参数，打开浏览器的 URL）。
    pub authorize_url: String,
}

/// 启动登录：起本机回调服务 + 打开浏览器（PKCE 流程）。
///
/// client_id 固定 `100ask-desktop`，无需 client_secret。每次登录重新生成 state 与 PKCE
/// 密钥对：challenge 放进授权 URL，verifier 留在本机会话供换 token 校验。
/// 命令立即返回，登录结果经 `auth-login-result` 事件回传。
#[tauri::command]
pub async fn auth_login_start(
    app: AppHandle,
    state: State<'_, Arc<AuthState>>,
    diagnostics: State<'_, Arc<DiagnosticState>>,
) -> Result<LoginStartResult, String> {
    if state.is_login_pending() {
        cleanup_server(&state).await;
        log::warn!("检测到未完成的 100ask.net 登录，已清理并重新开始");
    }

    let cfg = &state.oauth;

    // 生成 state（防 CSRF）与 PKCE 密钥对（每次登录重新生成）。
    let oauth_state = random_string(32);
    let pkce = generate_pkce();

    // 在写入 pending 前完成所有可能同步失败的准备，避免错误路径遗留永久登录锁。
    let (host, port) = cfg.bind_addr()?;
    let authorize_url = cfg.build_authorize_url(&oauth_state, &pkce.challenge)?;
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| format!("绑定回调服务 {host}:{port} 失败: {e}"))?;
    state.set_pending(super::state::PendingLogin {
        state: oauth_state.clone(),
        code_verifier: pkce.verifier,
    });

    // 关停信号：回调 handler / 取消 / 超时任一方 take 后发送，触发 axum graceful shutdown。
    let (tx, rx) = oneshot::channel::<()>();
    let shutdown = Arc::new(Mutex::new(Some(tx)));
    let cancel = CancellationToken::new();

    let cs = CallbackState {
        auth: state.inner().clone(),
        diagnostics: diagnostics.inner().clone(),
        app: app.clone(),
    };
    let app_router = router(cs);

    let join = tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, app_router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    state.set_server(RunningAuthServer {
        shutdown: shutdown.clone(),
        cancel,
        join,
    });

    // 登录超时守护：到点若回调服务仍在跑，emit 超时事件并关停。
    let auth_clone = state.inner().clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LOGIN_TIMEOUT).await;
        // 只回收本轮的残留；取消后重试的登录拥有自己的完整截止时间。
        let (server, pending) = auth_clone.take_expired_login(&oauth_state, &shutdown);
        if server.is_some() || pending.is_some() {
            let _ = app_clone.emit(
                "auth-login-result",
                LoginResultEvent {
                    success: false,
                    user: None,
                    error: Some("登录超时，请重试".into()),
                },
            );
            if let Some(srv) = server {
                srv.cancel.cancel();
                if let Some(tx) = srv.shutdown.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(());
                }
                let _ = srv.join.await;
            }
            log::warn!("100ask.net 登录超时，已关停回调服务");
        }
    });

    log::info!("100ask.net 登录已启动，回调服务监听 {host}:{port}");
    Ok(LoginStartResult {
        port,
        redirect_uri: cfg.redirect_uri.clone(),
        authorize_url,
    })
}

/// 查询当前登录用户（未登录返回 null）。
#[tauri::command]
pub async fn auth_get_user(
    app: AppHandle,
    state: State<'_, Arc<AuthState>>,
    diagnostics: State<'_, Arc<DiagnosticState>>,
) -> Result<Option<UserInfo>, String> {
    let auth = state.inner().clone();
    let user = auth.get_user().await?;
    if user.is_some() {
        crate::diagnostics::activate_for_authenticated_session(
            diagnostics.inner().clone(),
            auth,
            app,
        )?;
    }
    Ok(user)
}

/// 登出：清空账号会话，并异步撤销诊断授权、隔离本地队列。
#[tauri::command]
pub fn auth_logout(
    state: State<'_, Arc<AuthState>>,
    diagnostics: State<'_, Arc<DiagnosticState>>,
) -> Result<(), String> {
    let logout_result = state.clear_session();
    diagnostics.inner().clone().schedule_logout_reset();
    logout_result?;
    log::info!("100ask.net 已登出");
    Ok(())
}

/// 取消进行中的登录：清 pending + 关停回调服务。
#[tauri::command]
pub async fn auth_cancel_login(state: State<'_, Arc<AuthState>>) -> Result<(), String> {
    cleanup_server(&state).await;
    log::info!("100ask.net 登录已取消");
    Ok(())
}

/// 关停运行中的回调服务（撤销业务所有权，取消网络处理并释放监听端口）。
async fn cleanup_server(state: &State<'_, Arc<AuthState>>) {
    if let Some(srv) = state.cancel_login() {
        srv.cancel.cancel();
        if let Some(tx) = srv.shutdown.lock().ok().and_then(|mut g| g.take()) {
            let _ = tx.send(());
        }
        // The callback has already lost its claim and received its network
        // cancellation signal. Do not wait for HTTP keep-alive draining before
        // returning control to the user or allowing a retry on the fixed port.
        srv.join.abort();
        tokio::task::yield_now().await;
    }
}

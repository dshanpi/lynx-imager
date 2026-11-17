//! 登录共享状态。
//!
//! [`AuthState`] 由 Tauri 托管为 `Arc<AuthState>` 单例，跨命令与 axum 回调 handler 共享，
//! 持有：OAuth 配置、复用的 HTTP 客户端、待处理的 state（防 CSRF + 关联回调）、当前会话、
//! 以及运行中的回调服务句柄（用于回调完成或取消时关停）。

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use keyring::Entry;
use tauri::async_runtime::JoinHandle;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::config::{fetch_userinfo, refresh_access_token, OAuthConfig, Session, UserInfo};

const AUTH_KEYRING_SERVICE: &str = "com.dshanpi.lynx.auth";
const AUTH_KEYRING_ACCOUNT: &str = "oauth-session";
#[cfg(debug_assertions)]
const AUTH_TEST_KEYRING_PREFIX: &str = "com.dshanpi.lynx.auth.test";

/// 待处理的一次登录（持有随机 state 与 PKCE code_verifier，等回调回来校验）。
pub struct PendingLogin {
    /// 随机 state 串，与授权 URL 中带出的一致（防 CSRF）。
    pub state: String,
    /// PKCE code_verifier，换 token 时发回服务端校验。必须保密在本机会话。
    pub code_verifier: String,
}

/// 运行中的回调服务（关停信号 + 任务句柄）。
///
/// `shutdown` 用 `Arc<Mutex<Option<Sender>>>` 包裹，便于回调 handler 在收到回调后
/// 主动触发关停，也便于 `auth_cancel_login` 取消登录时关停。
pub struct RunningAuthServer {
    pub shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Cancels token/userinfo work owned by this exact login round.
    pub cancel: CancellationToken,
    pub join: JoinHandle<()>,
}

/// 登录模块状态（Tauri 托管单例）。
pub struct AuthState {
    /// OAuth 配置（从环境变量读取）。
    pub oauth: OAuthConfig,
    /// 复用的 HTTP 客户端（换 token / 取用户信息）。
    pub http: reqwest::Client,
    /// 待处理的登录（启动登录时写入，回调时校验并清空）。
    pending: Mutex<Option<PendingLogin>>,
    /// 已取得 PKCE、尚未提交终态的回调 state。取消/重试可在提交前清除它。
    callback_claim: Mutex<Option<String>>,
    /// 当前会话（登录成功后写入）。
    session: Mutex<Option<Session>>,
    /// Revision is read and advanced only while holding session.
    session_revision: AtomicU64,
    refresh_gate: tokio::sync::Mutex<()>,
    /// 运行中的回调服务句柄（登录进行中存在）。
    server: Mutex<Option<RunningAuthServer>>,
    #[cfg(test)]
    memory_only: bool,
}

impl AuthState {
    pub fn new() -> Self {
        let session = match load_session() {
            Ok(session) => session,
            Err(error) => {
                log::warn!("无法恢复 100ask.net 登录会话: {error}");
                None
            }
        };
        Self {
            oauth: OAuthConfig::new(),
            http: reqwest::Client::new(),
            pending: Mutex::new(None),
            callback_claim: Mutex::new(None),
            session: Mutex::new(session),
            session_revision: AtomicU64::new(0),
            refresh_gate: tokio::sync::Mutex::new(()),
            server: Mutex::new(None),
            #[cfg(test)]
            memory_only: false,
        }
    }

    /// 是否有登录正在进行（等待回调、处理中或回调服务仍在跑）。
    pub fn is_login_pending(&self) -> bool {
        self.pending.lock().map(|g| g.is_some()).unwrap_or(false)
            || self
                .callback_claim
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false)
            || self.server.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// 写入待处理 state（启动登录时调用）。
    pub fn set_pending(&self, pending: PendingLogin) {
        if let Ok(mut g) = self.pending.lock() {
            *g = Some(pending);
        }
    }

    /// 测试辅助：取出并清空待处理 state。
    #[cfg(test)]
    pub fn take_pending(&self) -> Option<PendingLogin> {
        self.pending.lock().ok().and_then(|mut g| g.take())
    }

    /// Atomically hand PKCE material to the matching callback while retaining a
    /// shared claim. Deadline, cancel and retry lock the same slots in the same
    /// order, so they cannot split ownership of one login.
    pub fn claim_login_callback(&self, expected_state: &str) -> Option<PendingLogin> {
        let mut pending = self.pending.lock().ok()?;
        let mut callback_claim = self.callback_claim.lock().ok()?;
        let server = self.server.lock().ok()?;
        if !pending
            .as_ref()
            .is_some_and(|login| login.state == expected_state)
            || callback_claim.is_some()
            || server.is_none()
        {
            return None;
        }
        let pending = pending.take()?;
        *callback_claim = Some(expected_state.to_owned());
        Some(pending)
    }

    /// Commit the callback's terminal ownership and take its exact service.
    /// A cancel/retry that won first clears the claim, so the callback cannot
    /// persist a session or emit a late terminal result.
    pub fn finish_login_callback(&self, expected_state: &str) -> Option<RunningAuthServer> {
        let mut callback_claim = self.callback_claim.lock().ok()?;
        let mut server = self.server.lock().ok()?;
        if callback_claim.as_deref() != Some(expected_state) || server.is_none() {
            return None;
        }
        callback_claim.take();
        server.take()
    }

    /// Clone the network cancellation signal only for the callback that owns the
    /// current processing claim. A concurrent cancel may take the server after
    /// this method returns; it then cancels the cloned token as well.
    pub fn callback_cancellation(&self, expected_state: &str) -> Option<CancellationToken> {
        let callback_claim = self.callback_claim.lock().ok()?;
        let server = self.server.lock().ok()?;
        if callback_claim.as_deref() != Some(expected_state) {
            return None;
        }
        server.as_ref().map(|running| running.cancel.clone())
    }

    /// Cancel the waiting or processing login and atomically take its service.
    pub fn cancel_login(&self) -> Option<RunningAuthServer> {
        let mut pending = self.pending.lock().ok()?;
        let mut callback_claim = self.callback_claim.lock().ok()?;
        let mut server = self.server.lock().ok()?;
        pending.take();
        callback_claim.take();
        server.take()
    }

    /// Persist and publish a new login under the same lock as logout/refresh.
    pub fn set_session(&self, session: Session) -> Result<(), String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|error| format!("AUTH_SESSION_LOCK:{error}"))?;
        self.store_session(&session)?;
        *guard = Some(session);
        self.session_revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn session_snapshot(&self) -> Result<(Option<Session>, u64), String> {
        let guard = self
            .session
            .lock()
            .map_err(|error| format!("AUTH_SESSION_LOCK:{error}"))?;
        Ok((guard.clone(), self.session_revision.load(Ordering::Relaxed)))
    }

    fn commit_refreshed_session(&self, revision: u64, session: Session) -> Result<bool, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|error| format!("AUTH_SESSION_LOCK:{error}"))?;
        if self.session_revision.load(Ordering::Relaxed) != revision {
            return Ok(false);
        }
        self.store_session(&session)?;
        *guard = Some(session);
        // Rotation stays within the same login ownership. Only an explicit
        // new login or logout changes the revision seen by queued readers.
        Ok(true)
    }

    /// Return only a usable current user. If an old refresh lost ownership,
    /// project the current login (or logout) rather than the stale result.
    pub async fn get_user(&self) -> Result<Option<UserInfo>, String> {
        match self.refresh_session().await {
            Ok(session) => Ok(session.map(|session| session.user)),
            Err(error) if error.starts_with("AUTH_SESSION_CHANGED:") => {
                let (current, _) = self.session_snapshot()?;
                Ok(current
                    .filter(Session::has_valid_access_token)
                    .map(|session| session.user))
            }
            Err(error) => Err(error),
        }
    }

    /// Bearers never cross a login/logout revision while refresh is in flight.
    pub async fn get_access_token(&self) -> Result<String, String> {
        self.refresh_session()
            .await?
            .map(|session| session.access_token)
            .ok_or_else(|| "LINK_AUTH_REQUIRED:请先登录 100ASK 账号".to_owned())
    }

    async fn refresh_session(&self) -> Result<Option<Session>, String> {
        let (initial, initial_revision) = self.session_snapshot()?;
        if initial.as_ref().is_none_or(Session::has_valid_access_token) {
            return Ok(initial);
        }
        // Queue both public readers behind one refresh; re-read only after the
        // gate is acquired so a rotated token is never replayed by a waiter.
        let _refresh = self.refresh_gate.lock().await;
        let (snapshot, revision) = self.session_snapshot()?;
        if revision != initial_revision {
            return Err("AUTH_SESSION_CHANGED:登录状态已变化，请重试".into());
        }
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        if snapshot.has_valid_access_token() {
            return Ok(Some(snapshot));
        }

        let refreshed = async {
            let refresh_token = snapshot.refresh_token.as_deref().ok_or(())?;
            let token = refresh_access_token(&self.http, &self.oauth, refresh_token)
                .await
                .map_err(|_| ())?;
            if token.access_token.trim().is_empty() || token.expires_in <= 0 {
                return Err(());
            }
            let user = fetch_userinfo(&self.http, &self.oauth, &token.access_token)
                .await
                .map_err(|_| ())?;
            // A valid response may not change account identity during refresh.
            if user.id != snapshot.user.id {
                return Err(());
            }
            Ok(Session::with_refresh_token(
                user,
                token.access_token,
                token.expires_in,
                token
                    .refresh_token
                    .or_else(|| snapshot.refresh_token.clone()),
            ))
        }
        .await;

        match refreshed {
            Ok(session) => {
                if !self.commit_refreshed_session(revision, session.clone())? {
                    return Err("AUTH_SESSION_CHANGED:登录状态已变化，请重试".into());
                }
                Ok(Some(session))
            }
            Err(()) => {
                let mut remembered = snapshot;
                remembered.scrub_expired_access_token();
                if !self.commit_refreshed_session(revision, remembered)? {
                    return Err("AUTH_SESSION_CHANGED:登录状态已变化，请重试".into());
                }
                Err("AUTH_TOKEN_EXPIRED:登录已过期，请重新登录".into())
            }
        }
    }

    /// Revoke in-memory ownership even if the OS keyring cannot be deleted.
    pub fn clear_session(&self) -> Result<(), String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|error| format!("AUTH_SESSION_LOCK:{error}"))?;
        *guard = None;
        self.session_revision.fetch_add(1, Ordering::Relaxed);
        self.delete_stored_session()
    }

    fn store_session(&self, session: &Session) -> Result<(), String> {
        #[cfg(test)]
        if self.memory_only {
            return Ok(());
        }
        persist_session(session)
    }

    fn delete_stored_session(&self) -> Result<(), String> {
        #[cfg(test)]
        if self.memory_only {
            return Ok(());
        }
        delete_session()
    }

    /// 记录运行中的回调服务。
    pub fn set_server(&self, server: RunningAuthServer) {
        if let Ok(mut g) = self.server.lock() {
            *g = Some(server);
        }
    }

    /// 测试辅助：取出回调服务，调用方负责关停并等待任务结束。
    #[cfg(test)]
    pub fn take_server(&self) -> Option<RunningAuthServer> {
        self.server.lock().ok().and_then(|mut g| g.take())
    }

    /// A detached timeout belongs to one login, even after cancellation/retry.
    /// Hold both slots while checking ownership: neither an old pending state nor
    /// an old server handle may authorize taking a newer login's other slot.
    pub fn take_expired_login(
        &self,
        expected_state: &str,
        shutdown: &Arc<Mutex<Option<oneshot::Sender<()>>>>,
    ) -> (Option<RunningAuthServer>, Option<PendingLogin>) {
        let Ok(mut pending) = self.pending.lock() else {
            return (None, None);
        };
        let Ok(callback_claim) = self.callback_claim.lock() else {
            return (None, None);
        };
        let Ok(mut server) = self.server.lock() else {
            return (None, None);
        };
        if callback_claim.is_some()
            || pending
                .as_ref()
                .is_some_and(|login| login.state != expected_state)
            || server
                .as_ref()
                .is_some_and(|running| !Arc::ptr_eq(&running.shutdown, shutdown))
        {
            return (None, None);
        }
        (server.take(), pending.take())
    }
}

fn session_entry() -> Result<Entry, String> {
    let service = session_service();
    Entry::new(&service, AUTH_KEYRING_ACCOUNT).map_err(|error| format!("AUTH_KEYRING:{error}"))
}

fn session_service() -> String {
    #[cfg(debug_assertions)]
    if let Ok(namespace) = std::env::var("LYNX_TEST_AUTH_KEYRING_NAMESPACE") {
        let namespace = namespace.trim();
        if !namespace.is_empty()
            && namespace.len() <= 64
            && namespace
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return format!("{AUTH_TEST_KEYRING_PREFIX}.{namespace}");
        }
        log::warn!("Ignored invalid debug-only OAuth keyring namespace");
    }
    AUTH_KEYRING_SERVICE.to_owned()
}

fn load_session() -> Result<Option<Session>, String> {
    let encoded = match session_entry()?.get_password() {
        Ok(encoded) => encoded,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(format!("AUTH_SESSION_LOAD:{error}")),
    };
    let mut session = serde_json::from_str::<Session>(&encoded)
        .map_err(|error| format!("AUTH_SESSION_DECODE:{error}"))?;
    if session.is_expired() {
        if session.scrub_expired_access_token() {
            persist_session(&session)?;
        }
        log::info!("已从系统凭据库恢复记住的 100ask.net 账号；访问令牌需要刷新");
    } else {
        log::info!("已从系统凭据库恢复 100ask.net 登录会话");
    }
    Ok(Some(session))
}

fn persist_session(session: &Session) -> Result<(), String> {
    let encoded =
        serde_json::to_string(session).map_err(|error| format!("AUTH_SESSION_ENCODE:{error}"))?;
    session_entry()?
        .set_password(&encoded)
        .map_err(|error| format!("AUTH_SESSION_STORE:{error}"))
}

fn delete_session() -> Result<(), String> {
    match session_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("AUTH_SESSION_DELETE:{error}")),
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_identity_is_stable_across_versions_and_reinstall() {
        assert_eq!(AUTH_KEYRING_SERVICE, "com.dshanpi.lynx.auth");
        assert_eq!(AUTH_KEYRING_ACCOUNT, "oauth-session");
        assert!(!AUTH_KEYRING_SERVICE.contains(env!("CARGO_PKG_VERSION")));
    }

    pub(super) fn state_with_login(
        state: &str,
    ) -> (AuthState, Arc<Mutex<Option<oneshot::Sender<()>>>>) {
        let (tx, rx) = oneshot::channel();
        let shutdown = Arc::new(Mutex::new(Some(tx)));
        (
            AuthState {
                memory_only: true,
                oauth: OAuthConfig::new(),
                http: reqwest::Client::new(),
                pending: Mutex::new(Some(PendingLogin {
                    state: state.into(),
                    code_verifier: "test-only-verifier".into(),
                })),
                callback_claim: Mutex::new(None),
                session: Mutex::new(None),
                session_revision: AtomicU64::new(0),
                refresh_gate: tokio::sync::Mutex::new(()),
                server: Mutex::new(Some(RunningAuthServer {
                    shutdown: shutdown.clone(),
                    cancel: CancellationToken::new(),
                    join: tauri::async_runtime::spawn(async move {
                        let _ = rx.await;
                    }),
                })),
            },
            shutdown,
        )
    }

    pub(super) async fn stop_server(server: RunningAuthServer) {
        if let Some(tx) = server.shutdown.lock().unwrap().take() {
            let _ = tx.send(());
        }
        server.join.await.unwrap();
    }

    #[tokio::test]
    async fn matching_callback_atomically_claims_pending_login() {
        let (state, _) = state_with_login("expected");
        let pending = state.claim_login_callback("expected").unwrap();
        assert_eq!(pending.state, "expected");
        assert_eq!(pending.code_verifier, "test-only-verifier");
        assert!(state.take_pending().is_none());
        let server = state.finish_login_callback("expected").unwrap();
        assert!(state.take_server().is_none());
        stop_server(server).await;
    }

    #[tokio::test]
    async fn wrong_callback_state_preserves_pending_login() {
        let (state, _) = state_with_login("expected");
        assert!(state.claim_login_callback("wrong").is_none());
        let pending = state.claim_login_callback("expected").unwrap();
        assert_eq!(pending.state, "expected");
        let server = state.finish_login_callback("expected").unwrap();
        stop_server(server).await;
    }

    #[tokio::test]
    async fn duplicate_callback_cannot_claim_pkce_twice() {
        let (state, _) = state_with_login("expected");
        state.claim_login_callback("expected").unwrap();
        assert!(state.claim_login_callback("expected").is_none());
        let server = state.finish_login_callback("expected").unwrap();
        stop_server(server).await;
    }

    async fn timeout_ownership_case(pending_owner: Option<bool>, server_owner: Option<bool>) {
        use super::*;
        let (expected_tx, expected_rx) = oneshot::channel();
        let expected = Arc::new(Mutex::new(Some(expected_tx)));
        let server = server_owner.map(|owned| {
            let (shutdown, rx) = if owned {
                (expected.clone(), expected_rx)
            } else {
                let (tx, rx) = oneshot::channel();
                (Arc::new(Mutex::new(Some(tx))), rx)
            };
            RunningAuthServer {
                shutdown,
                cancel: CancellationToken::new(),
                join: tauri::async_runtime::spawn(async move {
                    let _ = rx.await;
                }),
            }
        });
        let state = AuthState {
            memory_only: true,
            oauth: OAuthConfig::new(),
            http: reqwest::Client::new(),
            pending: Mutex::new(pending_owner.map(|owned| PendingLogin {
                state: if owned { "original" } else { "retry" }.into(),
                code_verifier: "test-only-verifier".into(),
            })),
            callback_claim: Mutex::new(None),
            session: Mutex::new(None),
            session_revision: AtomicU64::new(0),
            refresh_gate: tokio::sync::Mutex::new(()),
            server: Mutex::new(server),
        };
        let (expired_server, expired_pending) = state.take_expired_login("original", &expected);
        let replaced = pending_owner == Some(false) || server_owner == Some(false);
        assert_eq!(
            expired_server.is_some(),
            !replaced && server_owner.is_some()
        );
        assert_eq!(
            expired_pending.is_some(),
            !replaced && pending_owner.is_some()
        );
        let remaining_pending = state.take_pending();
        assert_eq!(
            remaining_pending.is_some(),
            replaced && pending_owner.is_some()
        );
        if let Some(pending) = remaining_pending {
            assert_eq!(
                pending.state,
                if pending_owner == Some(true) {
                    "original"
                } else {
                    "retry"
                }
            );
            assert_eq!(pending.code_verifier, "test-only-verifier");
        }
        let remaining_server = state.take_server();
        assert_eq!(
            remaining_server.is_some(),
            replaced && server_owner.is_some()
        );
        for running in [expired_server, remaining_server].into_iter().flatten() {
            assert_eq!(
                Arc::ptr_eq(&running.shutdown, &expected),
                server_owner == Some(true)
            );
            if let Some(tx) = running.shutdown.lock().unwrap().take() {
                let _ = tx.send(());
            }
            running.join.await.unwrap();
        }
    }

    #[tokio::test]
    async fn own_timeout_takes_pending_and_server() {
        timeout_ownership_case(Some(true), Some(true)).await;
    }

    #[tokio::test]
    async fn old_timeout_preserves_retried_login() {
        timeout_ownership_case(Some(false), Some(false)).await;
    }

    #[tokio::test]
    async fn callback_claim_prevents_timeout_from_reclaiming_its_server() {
        let (tx, rx) = oneshot::channel();
        let shutdown = Arc::new(Mutex::new(Some(tx)));
        let state = AuthState {
            memory_only: true,
            oauth: OAuthConfig::new(),
            http: reqwest::Client::new(),
            pending: Mutex::new(Some(PendingLogin {
                state: "original".into(),
                code_verifier: "test-only-verifier".into(),
            })),
            callback_claim: Mutex::new(None),
            session: Mutex::new(None),
            session_revision: AtomicU64::new(0),
            refresh_gate: tokio::sync::Mutex::new(()),
            server: Mutex::new(Some(RunningAuthServer {
                shutdown: shutdown.clone(),
                cancel: CancellationToken::new(),
                join: tauri::async_runtime::spawn(async move {
                    let _ = rx.await;
                }),
            })),
        };
        let claimed = state.claim_login_callback("original").unwrap();
        assert_eq!(claimed.code_verifier, "test-only-verifier");

        let (expired_server, expired_pending) = state.take_expired_login("original", &shutdown);
        let timeout_took_server = expired_server.is_some();
        assert!(expired_pending.is_none());

        let server = state.finish_login_callback("original").unwrap();
        stop_server(server).await;
        assert!(!timeout_took_server);
    }

    #[tokio::test]
    async fn cancel_can_win_before_callback_terminal_commit() {
        let (state, _) = state_with_login("original");
        state.claim_login_callback("original").unwrap();

        let cancelled_server = state.cancel_login();
        let cancellation_won = cancelled_server.is_some();
        assert!(state.finish_login_callback("original").is_none());

        stop_server(cancelled_server.unwrap()).await;
        assert!(cancellation_won);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn callback_and_timeout_have_exactly_one_owner_under_race() {
        use std::sync::Barrier;

        for _ in 0..64 {
            let (state, shutdown) = state_with_login("original");
            let barrier = Arc::new(Barrier::new(3));
            let (callback, timeout) = std::thread::scope(|scope| {
                let callback_barrier = barrier.clone();
                let callback_state = &state;
                let callback = scope.spawn(move || {
                    callback_barrier.wait();
                    callback_state
                        .claim_login_callback("original")
                        .map(|pending| (pending, callback_state.finish_login_callback("original")))
                });
                let timeout_barrier = barrier.clone();
                let timeout_state = &state;
                let timeout_shutdown = shutdown.clone();
                let timeout = scope.spawn(move || {
                    timeout_barrier.wait();
                    timeout_state.take_expired_login("original", &timeout_shutdown)
                });
                barrier.wait();
                (callback.join().unwrap(), timeout.join().unwrap())
            });

            let callback_won = callback.is_some();
            let timeout_won = timeout.0.is_some() || timeout.1.is_some();
            assert_ne!(callback_won, timeout_won);
            let server = callback
                .and_then(|(_, server)| server)
                .or(timeout.0)
                .unwrap();
            stop_server(server).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_and_callback_commit_have_exactly_one_owner_under_race() {
        use std::sync::Barrier;

        for _ in 0..64 {
            let (state, _) = state_with_login("original");
            state.claim_login_callback("original").unwrap();
            let barrier = Arc::new(Barrier::new(3));
            let (cancelled_server, callback_server) = std::thread::scope(|scope| {
                let cancel_barrier = barrier.clone();
                let cancel_state = &state;
                let cancel = scope.spawn(move || {
                    cancel_barrier.wait();
                    cancel_state.cancel_login()
                });
                let callback_barrier = barrier.clone();
                let callback_state = &state;
                let callback = scope.spawn(move || {
                    callback_barrier.wait();
                    callback_state.finish_login_callback("original")
                });
                barrier.wait();
                (cancel.join().unwrap(), callback.join().unwrap())
            });

            assert_ne!(cancelled_server.is_some(), callback_server.is_some());
            stop_server(cancelled_server.or(callback_server).unwrap()).await;
        }
    }

    #[tokio::test]
    async fn old_timeout_preserves_retried_server_after_pending_is_claimed() {
        timeout_ownership_case(None, Some(false)).await;
    }

    #[tokio::test]
    async fn own_timeout_reclaims_pending_without_server() {
        timeout_ownership_case(Some(true), None).await;
    }

    #[tokio::test]
    async fn old_timeout_preserves_new_pending_without_server() {
        timeout_ownership_case(Some(false), None).await;
    }

    #[tokio::test]
    async fn cancelled_or_completed_login_has_no_duplicate_timeout() {
        timeout_ownership_case(None, None).await;
    }

    #[tokio::test]
    async fn pending_ownership_does_not_authorize_taking_a_new_server() {
        timeout_ownership_case(Some(true), Some(false)).await;
    }

    #[tokio::test]
    async fn server_ownership_does_not_authorize_taking_new_pending() {
        timeout_ownership_case(Some(false), Some(true)).await;
    }
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;

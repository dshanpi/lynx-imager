use super::tests::{state_with_login, stop_server};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Stub {
    requests: Arc<AtomicUsize>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    fail: bool,
}

fn session(name: &str, expired: bool) -> Session {
    Session {
        user: UserInfo {
            id: if name == "old" { 1 } else { 2 },
            username: name.into(),
            name: name.into(),
            email: format!("{name}@example.invalid"),
            email_verified: true,
            avatar_url: String::new(),
        },
        access_token: format!("test-only-{name}"),
        expires_in: if expired { 0 } else { 3600 },
        issued_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        refresh_token: Some("test-only-refresh".into()),
    }
}

async fn fixture(fail: bool) -> (Arc<AuthState>, Stub, tokio::task::JoinHandle<()>) {
    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };
    let stub = Stub {
        requests: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
        fail,
    };
    let router = Router::new()
        .route(
            "/token",
            post(|State(stub): State<Stub>| async move {
                stub.requests.fetch_add(1, Ordering::SeqCst);
                stub.entered.add_permits(1);
                stub.release.acquire().await.unwrap().forget();
                if stub.fail {
                    return (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error":"expired"})),
                    );
                }
                (
                    axum::http::StatusCode::OK,
                    Json(serde_json::json!({
                        "access_token":"test-only-refreshed", "expires_in":3600,
                        "refresh_token":"test-only-rotated"
                    })),
                )
            }),
        )
        .route("/user", get(|| async { Json(session("old", false).user) }))
        .with_state(stub.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut state = state_with_login("unused").0;
    stop_server(state.cancel_login().unwrap()).await;
    state.oauth.token_url = format!("http://{address}/token");
    state.oauth.userinfo_url = format!("http://{address}/user");
    state.set_session(session("old", true)).unwrap();
    (Arc::new(state), stub, server)
}

async fn entered(stub: &Stub) {
    tokio::time::timeout(std::time::Duration::from_secs(3), stub.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}

#[tokio::test]
async fn concurrent_user_and_token_share_one_refresh() {
    let (state, stub, server) = fixture(false).await;
    let first = tokio::spawn({
        let state = state.clone();
        async move { state.get_user().await }
    });
    entered(&stub).await;
    let second = tokio::spawn({
        let state = state.clone();
        async move { state.get_access_token().await }
    });
    // Both requests are released; count, not elapsed time, proves deduplication.
    stub.release.add_permits(2);
    assert_eq!(first.await.unwrap().unwrap().unwrap().username, "old");
    assert_eq!(second.await.unwrap().unwrap(), "test-only-refreshed");
    assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn refresh_after_logout_cannot_restore_user_or_bearer() {
    let (state, stub, server) = fixture(false).await;
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_access_token().await }
    });
    entered(&stub).await;
    state.clear_session().unwrap();
    stub.release.add_permits(1);
    assert!(refresh.await.unwrap().is_err());
    assert!(state.session.lock().unwrap().is_none());
    server.abort();
}

#[tokio::test]
async fn refresh_success_cannot_replace_new_login() {
    let (state, stub, server) = fixture(false).await;
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_user().await }
    });
    entered(&stub).await;
    state.set_session(session("new", false)).unwrap();
    stub.release.add_permits(1);
    let result = refresh.await.unwrap();
    assert!(result.is_err() || result.unwrap().is_none_or(|user| user.username == "new"));
    assert_eq!(
        state
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .user
            .username,
        "new"
    );
    server.abort();
}

#[tokio::test]
async fn refresh_failure_cannot_scrub_new_login() {
    let (state, stub, server) = fixture(true).await;
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_user().await }
    });
    entered(&stub).await;
    state.set_session(session("new", false)).unwrap();
    stub.release.add_permits(1);
    let _ = refresh.await.unwrap();
    let guard = state.session.lock().unwrap();
    let current = guard.as_ref().unwrap();
    assert_eq!(current.user.username, "new");
    assert_eq!(current.access_token, "test-only-new");
    server.abort();
}

#[tokio::test]
async fn owned_refresh_failure_scrubs_expired_bearer_and_keeps_identity() {
    let (state, stub, server) = fixture(true).await;
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_user().await }
    });
    entered(&stub).await;
    stub.release.add_permits(1);
    assert!(refresh
        .await
        .unwrap()
        .unwrap_err()
        .starts_with("AUTH_TOKEN_EXPIRED:"));
    let current = state.session.lock().unwrap().clone().unwrap();
    assert_eq!(current.user.username, "old");
    assert!(current.access_token.is_empty());
    assert_eq!(current.refresh_token.as_deref(), Some("test-only-refresh"));
    server.abort();
}

#[tokio::test]
async fn refresh_cannot_change_account_identity() {
    let (state, stub, server) = fixture(false).await;
    let mut changed = session("old", true);
    changed.user.id = 999;
    state.set_session(changed).unwrap();
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_access_token().await }
    });
    entered(&stub).await;
    stub.release.add_permits(1);
    assert!(refresh.await.unwrap().is_err());
    assert_eq!(state.session.lock().unwrap().as_ref().unwrap().user.id, 999);
    server.abort();
}

#[tokio::test]
async fn current_session_reads_do_not_call_refresh() {
    let (state, stub, server) = fixture(false).await;
    state.set_session(session("new", false)).unwrap();
    assert_eq!(state.get_user().await.unwrap().unwrap().username, "new");
    assert_eq!(state.get_access_token().await.unwrap(), "test-only-new");
    state.clear_session().unwrap();
    assert!(state.get_user().await.unwrap().is_none());
    assert!(state
        .get_access_token()
        .await
        .unwrap_err()
        .starts_with("LINK_AUTH_REQUIRED:"));
    assert_eq!(stub.requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn logout_is_observable_without_waiting_for_old_refresh() {
    let (state, stub, server) = fixture(false).await;
    let refresh = tokio::spawn({
        let state = state.clone();
        async move { state.get_access_token().await }
    });
    entered(&stub).await;
    state.clear_session().unwrap();
    let observed =
        tokio::time::timeout(std::time::Duration::from_millis(200), state.get_user()).await;
    // Release before asserting so the fixture never leaves a pending request.
    stub.release.add_permits(1);
    let _ = refresh.await.unwrap();
    assert!(
        observed.is_ok(),
        "logout projection waited for stale network work"
    );
    assert!(observed.unwrap().unwrap().is_none());
    server.abort();
}

#[tokio::test]
async fn queued_token_reader_cannot_switch_to_new_account() {
    let (state, _, server) = fixture(false).await;
    let gate = state.refresh_gate.lock().await;
    let token = state.get_access_token();
    tokio::pin!(token);
    assert!(futures_util::poll!(token.as_mut()).is_pending());
    state.set_session(session("new", false)).unwrap();
    drop(gate);
    assert!(
        token.await.is_err(),
        "queued request received another account's bearer"
    );
    server.abort();
}

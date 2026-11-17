//! Read-only, channel-scoped loopback reachability. Never starts a listener or tunnel.
use super::{AppError, GatewayState};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayChannelStatus {
    pub project_id: i64,
    pub device_id: i64,
    pub running: bool,
    pub verified: bool,
    pub port: Option<u16>,
    pub endpoint: Option<String>,
    pub message: String,
}

fn snapshot(
    state: &GatewayState,
    project_id: i64,
    device_id: i64,
) -> Result<Option<(u16, u64)>, AppError> {
    let guard = state
        .runtimes
        .lock()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let mut matching = guard.iter().filter(|(_, runtime)| {
        !runtime.cancellation_token.is_cancelled()
            && runtime
                .channel_scope
                .as_ref()
                .is_some_and(|s| s.project_id == project_id && s.device_id == device_id)
    });
    let first = matching
        .next()
        .map(|(port, runtime)| (*port, runtime.generation));
    // Ambiguous generations must not advertise an arbitrary address.
    Ok(if matching.next().is_none() {
        first
    } else {
        None
    })
}

pub(super) async fn inspect(
    state: &GatewayState,
    project_id: i64,
    device_id: i64,
) -> Result<GatewayChannelStatus, AppError> {
    let mut status = GatewayChannelStatus {
        project_id,
        device_id,
        running: false,
        verified: false,
        port: None,
        endpoint: None,
        message: "当前单板通道的本机 MCP 服务未运行".into(),
    };
    let Some((port, generation)) = snapshot(state, project_id, device_id)? else {
        return Ok(status);
    };
    status.running = true;
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let response = client
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await;
    let verified = match response {
        Ok(response) if response.status().is_success() => {
            response.json::<Value>().await.ok().is_some_and(|v| {
                v["status"] == "ok" && v["product"] == "LYNX" && v["mcp"] == "/mcp"
            })
        }
        _ => false,
    };
    // Re-check ownership after I/O: the same port may now belong to another board/generation.
    if snapshot(state, project_id, device_id)? != Some((port, generation)) {
        status.running = false;
        status.message = "本机 MCP 服务已变化，等待重新验证".into();
        return Ok(status);
    }
    status.verified = verified;
    if verified {
        status.port = Some(port);
        status.endpoint = Some(format!("http://127.0.0.1:{port}/mcp"));
        status.message = "本机 HTTP 访问验证通过（不代表远端隧道可用）".into();
    } else {
        status.message = "监听器存在，但本机 HTTP 访问验证未通过".into();
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{ChannelScope, GatewayRuntime};
    use axum::{routing::get, Json, Router};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    fn insert(state: &GatewayState, port: u16, generation: u64) {
        state.runtimes.lock().unwrap().insert(
            port,
            GatewayRuntime {
                cancellation_token: CancellationToken::new(),
                generation,
                channel_scope: Some(ChannelScope {
                    project_id: 7,
                    device_id: 4,
                    project_name: "SDK".into(),
                    mcp_server_name: "lynx-test".into(),
                    chip_vendor: "allwinner".into(),
                    flasher_kind: "openixcli".into(),
                    flasher_path: String::new(),
                    serial_port: String::new(),
                    usb_binding: String::new(),
                    adb_serial: String::new(),
                    camera_binding: String::new(),
                }),
            },
        );
    }

    async fn server(router: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (port, handle)
    }

    #[tokio::test]
    async fn scoped_runtime_uses_actual_port_and_verifies_local_http() {
        let (port, server) = server(Router::new().route(
            "/healthz",
            get(|| async { Json(json!({"status":"ok", "product":"LYNX", "mcp":"/mcp"})) }),
        ))
        .await;
        let state = GatewayState::default();
        insert(&state, port, 1);
        let status = inspect(&state, 7, 4).await.unwrap();
        assert!(status.running && status.verified);
        assert_eq!(status.port, Some(port));
        assert_eq!(
            status.endpoint,
            Some(format!("http://127.0.0.1:{port}/mcp"))
        );
        for (project, device) in [(8, 4), (7, 5)] {
            let wrong = inspect(&state, project, device).await.unwrap();
            assert!(!wrong.running && !wrong.verified && wrong.endpoint.is_none());
        }
        server.abort();
    }

    #[tokio::test]
    async fn absent_cancelled_or_ambiguous_runtime_has_no_address() {
        let state = GatewayState::default();
        assert!(inspect(&state, 7, 4).await.unwrap().endpoint.is_none());
        insert(&state, 9241, 1);
        state
            .runtimes
            .lock()
            .unwrap()
            .get(&9241)
            .unwrap()
            .cancellation_token
            .cancel();
        assert!(!inspect(&state, 7, 4).await.unwrap().running);
        insert(&state, 9241, 2);
        insert(&state, 9242, 3);
        assert!(!inspect(&state, 7, 4).await.unwrap().running);
    }

    #[tokio::test]
    async fn another_http_service_cannot_be_advertised_as_lynx() {
        let (port, server) = server(Router::new().route(
            "/healthz",
            get(|| async { Json(json!({"status":"ok", "product":"OTHER", "mcp":"/mcp"})) }),
        ))
        .await;
        let state = GatewayState::default();
        insert(&state, port, 1);
        let status = inspect(&state, 7, 4).await.unwrap();
        assert!(status.running && !status.verified);
        assert!(status.endpoint.is_none() && status.port.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn old_health_response_cannot_verify_replaced_generation() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (seen, go) = (entered.clone(), release.clone());
        let (port, server) = server(Router::new().route(
            "/healthz",
            get(move || {
                let (seen, go) = (seen.clone(), go.clone());
                async move {
                    seen.notify_one();
                    go.notified().await;
                    Json(json!({"status":"ok", "product":"LYNX", "mcp":"/mcp"}))
                }
            }),
        ))
        .await;
        let state = Arc::new(GatewayState::default());
        insert(&state, port, 1);
        let querying = state.clone();
        let query = tokio::spawn(async move { inspect(&querying, 7, 4).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        insert(&state, port, 2);
        release.notify_one();
        let status = query.await.unwrap();
        assert!(!status.running && !status.verified && status.endpoint.is_none());
        server.abort();
    }
}

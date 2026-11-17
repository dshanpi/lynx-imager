//! Loopback HTTP/MCP routing composition.
//!
//! Gateway lifecycle, capability implementations, and interaction storage stay
//! in the parent module. This module only wires their transport endpoints.

use std::{
    collections::VecDeque,
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::Path as AxumPath,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use rmcp::transport::{
    streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService},
    StreamableHttpServerConfig,
};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use super::{ChannelScope, GatewayInteraction, LynxMcp, SerialSessions};

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "product": "LYNX",
        "apiVersion": "0.2",
        "version": env!("CARGO_PKG_VERSION"),
        "mcp": "/mcp"
    }))
}

pub(super) fn gateway_router(
    cancellation_token: &CancellationToken,
    app_handle: Option<AppHandle>,
    interactions: Arc<Mutex<VecDeque<GatewayInteraction>>>,
    next_interaction_id: Arc<AtomicU64>,
    channel_scope: Option<ChannelScope>,
) -> Router {
    let webhook_app = app_handle.clone();
    let webhook_scope = channel_scope.clone();
    let serial = SerialSessions::default();
    let service: StreamableHttpService<LynxMcp, LocalSessionManager> = StreamableHttpService::new(
        move || {
            Ok(LynxMcp::with_serial(
                app_handle.clone(),
                interactions.clone(),
                next_interaction_id.clone(),
                channel_scope.clone(),
                serial.clone(),
            ))
        },
        Default::default(),
        StreamableHttpServerConfig::default()
            .with_sse_keep_alive(Some(Duration::from_secs(15)))
            .with_cancellation_token(cancellation_token.child_token()),
    );

    let router = Router::new()
        .route("/healthz", get(health))
        .nest_service("/mcp", service);
    if let (Some(app), Some(scope)) = (webhook_app, webhook_scope) {
        router.route(
            "/supervisor/webhook/{task_id}",
            post(
                move |AxumPath(task_id): AxumPath<i64>, headers: HeaderMap| {
                    let app = app.clone();
                    let device_id = scope.device_id;
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok());
                        match crate::supervisor::scheduler::trigger_webhook(
                            &app,
                            task_id,
                            device_id,
                            authorization,
                        ) {
                            Ok(started_task_id) => (
                                StatusCode::ACCEPTED,
                                Json(json!({"status":"started", "taskId":started_task_id})),
                            ),
                            Err(error) => {
                                let status = if error == "SUPERVISOR_WEBHOOK_UNAUTHORIZED" {
                                    StatusCode::UNAUTHORIZED
                                } else {
                                    StatusCode::CONFLICT
                                };
                                (status, Json(json!({"status":"rejected", "error":error})))
                            }
                        }
                    }
                },
            ),
        )
    } else {
        router
    }
}

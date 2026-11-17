use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{routing::get, Json, Router};
use rmcp::transport::{
    streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService},
    StreamableHttpServerConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::app_error::AppError;

use super::tools::McuMcpTools;

const DEFAULT_MCU_MCP_PORT: u16 = 8767;

struct Runtime {
    cancellation: CancellationToken,
    board_id: String,
}

pub struct McuMcpGatewayState {
    runtimes: Arc<Mutex<HashMap<u16, Runtime>>>,
    interactions: Arc<Mutex<VecDeque<McuMcpInteraction>>>,
    next_interaction_id: Arc<AtomicU64>,
}

impl Default for McuMcpGatewayState {
    fn default() -> Self {
        Self {
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            interactions: Arc::new(Mutex::new(VecDeque::with_capacity(300))),
            next_interaction_id: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McuMcpInteraction {
    pub id: u64,
    pub timestamp_ms: u64,
    pub board_id: String,
    pub tool: String,
    pub category: String,
    pub summary: String,
}

pub(super) fn record_interaction(
    app: &AppHandle,
    interactions: &Arc<Mutex<VecDeque<McuMcpInteraction>>>,
    next_id: &Arc<AtomicU64>,
    board_id: &str,
    tool: &str,
    category: &str,
    summary: impl Into<String>,
) {
    let interaction = McuMcpInteraction {
        id: next_id.fetch_add(1, Ordering::SeqCst) + 1,
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        board_id: board_id.to_owned(),
        tool: tool.to_owned(),
        category: category.to_owned(),
        summary: summary.into(),
    };
    if let Ok(mut items) = interactions.lock() {
        if items.len() >= 300 {
            items.pop_front();
        }
        items.push_back(interaction.clone());
    }
    let _ = app.emit("mcu-mcp-interaction", interaction);
}

pub(super) fn is_running(
    state: &McuMcpGatewayState,
    port: u16,
    board_id: &str,
) -> Result<bool, String> {
    state
        .runtimes
        .lock()
        .map(|runtimes| {
            runtimes
                .get(&port)
                .is_some_and(|runtime| runtime.board_id == board_id)
        })
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McuMcpStartRequest {
    pub board_id: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McuMcpGatewayStatus {
    pub running: bool,
    pub channel_kind: &'static str,
    pub board_id: String,
    pub port: u16,
    pub endpoint: String,
}

impl McuMcpGatewayStatus {
    fn new(running: bool, board_id: String, port: u16) -> Self {
        Self {
            running,
            channel_kind: "mcu",
            board_id,
            port,
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
        }
    }
}

#[tauri::command]
pub async fn mcu_mcp_gateway_start(
    request: McuMcpStartRequest,
    app: AppHandle,
    state: State<'_, McuMcpGatewayState>,
) -> Result<McuMcpGatewayStatus, AppError> {
    crate::mcu::boards::board_by_id(&app, &request.board_id)
        .map_err(|error| AppError::internal(error.to_string()))?;
    let port = request.port.unwrap_or(DEFAULT_MCU_MCP_PORT);
    if port == 0 {
        return Err(AppError::internal("MCU MCP port must be non-zero"));
    }

    {
        let runtimes = state
            .runtimes
            .lock()
            .map_err(|error| AppError::internal(error.to_string()))?;
        if let Some(runtime) = runtimes.get(&port) {
            if runtime.board_id == request.board_id {
                return Ok(McuMcpGatewayStatus::new(true, request.board_id, port));
            }
            return Err(AppError::internal(format!(
                "MCU MCP 端口 {port} 已绑定另一块板卡；请为每块 MCU 单板分配独立端口"
            )));
        }
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| AppError::internal(format!("MCU MCP bind failed: {error}")))?;
    let cancellation = CancellationToken::new();
    let serial_ownership = Arc::new(Mutex::new(HashSet::new()));
    let app_for_service = app.clone();
    let board_for_service = request.board_id.clone();
    let service: StreamableHttpService<McuMcpTools, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let interactions = state.interactions.clone();
                let next_id = state.next_interaction_id.clone();
                move || {
                    Ok(McuMcpTools::new(
                        app_for_service.clone(),
                        board_for_service.clone(),
                        serial_ownership.clone(),
                        interactions.clone(),
                        next_id.clone(),
                    ))
                }
            },
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(Some(Duration::from_secs(15)))
                .with_cancellation_token(cancellation.child_token()),
        );
    async fn health() -> Json<serde_json::Value> {
        Json(json!({"status":"ok", "channelKind":"mcu"}))
    }
    let router = Router::new()
        .route("/healthz", get(health))
        .nest_service("/mcp", service);
    let shutdown = cancellation.clone();
    let runtimes = state.runtimes.clone();
    let runtime_board = request.board_id.clone();
    state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?
        .insert(
            port,
            Runtime {
                cancellation,
                board_id: request.board_id.clone(),
            },
        );
    tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await;
        if let Ok(mut current) = runtimes.lock() {
            if current
                .get(&port)
                .is_some_and(|runtime| runtime.board_id == runtime_board)
            {
                current.remove(&port);
            }
        }
        if let Err(error) = result {
            log::error!("MCU MCP gateway stopped: {error}");
        }
    });
    Ok(McuMcpGatewayStatus::new(true, request.board_id, port))
}

#[tauri::command]
pub fn mcu_mcp_list_interactions(
    state: State<'_, McuMcpGatewayState>,
    limit: Option<usize>,
) -> Result<Vec<McuMcpInteraction>, AppError> {
    let items = state
        .interactions
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(items
        .iter()
        .rev()
        .take(limit.unwrap_or(200).clamp(1, 300))
        .cloned()
        .collect())
}

#[tauri::command]
pub fn mcu_mcp_clear_interactions(
    state: State<'_, McuMcpGatewayState>,
    board_id: Option<String>,
) -> Result<(), AppError> {
    let mut interactions = state
        .interactions
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    if let Some(board_id) = board_id.filter(|value| !value.trim().is_empty()) {
        interactions.retain(|item| item.board_id != board_id);
    } else {
        interactions.clear();
    }
    Ok(())
}

#[tauri::command]
pub fn mcu_mcp_gateway_stop(
    port: Option<u16>,
    state: State<'_, McuMcpGatewayState>,
) -> Result<McuMcpGatewayStatus, AppError> {
    let port = port.unwrap_or(DEFAULT_MCU_MCP_PORT);
    let runtime = state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?
        .remove(&port);
    let board_id = runtime
        .as_ref()
        .map(|runtime| runtime.board_id.clone())
        .unwrap_or_default();
    if let Some(runtime) = runtime {
        runtime.cancellation.cancel();
    }
    Ok(McuMcpGatewayStatus::new(false, board_id, port))
}

#[tauri::command]
pub fn mcu_mcp_gateway_status(
    port: Option<u16>,
    state: State<'_, McuMcpGatewayState>,
) -> Result<McuMcpGatewayStatus, AppError> {
    let port = port.unwrap_or(DEFAULT_MCU_MCP_PORT);
    let runtimes = state
        .runtimes
        .lock()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(match runtimes.get(&port) {
        Some(runtime) => McuMcpGatewayStatus::new(true, runtime.board_id.clone(), port),
        None => McuMcpGatewayStatus::new(false, String::new(), port),
    })
}

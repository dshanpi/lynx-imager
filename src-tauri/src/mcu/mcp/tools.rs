use std::{
    collections::{HashSet, VecDeque},
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::{Duration, Instant},
};

use rmcp::{
    handler::server::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::serial::SerialState;
use crate::skills::{SkillDomain, SkillGetRequest, SkillReadRequest, SkillSearchRequest};

use super::super::{
    boards::{board_by_id, McuBoardConfig, ProbeBackend},
    commands::{
        publish_mcu_intent, publish_mcu_result, target_action_name, McuAuditActor, TargetAction,
        TargetStatus,
    },
    jlink::{self, JLinkActionRequest, JLinkConnection},
    openocd::{self, OpenOcdActionRequest, OpenOcdConnection},
    pyocd::{self, PyOcdActionRequest, PyOcdConnection},
    McuState,
};
use super::gateway::{record_interaction, McuMcpInteraction};

#[derive(Clone)]
pub(super) struct McuMcpTools {
    tool_router: ToolRouter<Self>,
    app: AppHandle,
    board_id: String,
    mcp_owned_serial: Arc<Mutex<HashSet<String>>>,
    interactions: Arc<Mutex<VecDeque<McuMcpInteraction>>>,
    next_interaction_id: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UartReadRequest {
    /// Maximum bytes to drain from the board UART, capped at 65536.
    max_bytes: Option<usize>,
    /// Wait for data for up to this many milliseconds, capped at 5000.
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UartWriteRequest {
    /// UTF-8 data sent to the configured board UART.
    data: String,
    /// Append CRLF after data.
    append_crlf: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum McuTargetAction {
    ResetRun,
    ResetHalt,
    Run,
    Halt,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TargetActionRequest {
    action: McuTargetAction,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PowerControlRequest {
    /// true powers the board on; false powers it off.
    on: bool,
}

impl McuMcpTools {
    pub(super) fn new(
        app: AppHandle,
        board_id: String,
        mcp_owned_serial: Arc<Mutex<HashSet<String>>>,
        interactions: Arc<Mutex<VecDeque<McuMcpInteraction>>>,
        next_interaction_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            app,
            board_id,
            mcp_owned_serial,
            interactions,
            next_interaction_id,
        }
    }

    fn record(&self, tool: &str, category: &str, summary: impl Into<String>) {
        record_interaction(
            &self.app,
            &self.interactions,
            &self.next_interaction_id,
            &self.board_id,
            tool,
            category,
            summary,
        );
    }

    fn board(&self) -> Result<McuBoardConfig, String> {
        board_by_id(&self.app, &self.board_id).map_err(|error| error.to_string())
    }

    fn connection(board: &McuBoardConfig) -> OpenOcdConnection {
        OpenOcdConnection {
            executable_path: (!board.tool_path.trim().is_empty()).then(|| board.tool_path.clone()),
            probe_serial: (!board.probe_serial.trim().is_empty())
                .then(|| board.probe_serial.clone()),
            frequency_khz: board.swd_frequency_khz,
            interface_config: board.openocd_interface_config.clone(),
            target_config: board.openocd_target_config.clone(),
            board_config: board.openocd_board_config.clone(),
        }
    }

    fn pyocd_connection(board: &McuBoardConfig) -> PyOcdConnection {
        PyOcdConnection {
            executable_path: (!board.tool_path.trim().is_empty()).then(|| board.tool_path.clone()),
            probe_serial: (!board.probe_serial.trim().is_empty())
                .then(|| board.probe_serial.clone()),
            frequency_khz: board.swd_frequency_khz,
            target: board.pyocd_target.clone(),
            pack_path: board.pyocd_pack_path.clone(),
        }
    }

    fn jlink_connection(board: &McuBoardConfig) -> JLinkConnection {
        JLinkConnection {
            executable_path: (!board.tool_path.trim().is_empty()).then(|| board.tool_path.clone()),
            probe_serial: (!board.probe_serial.trim().is_empty())
                .then(|| board.probe_serial.clone()),
            frequency_khz: board.swd_frequency_khz,
            device: board.jlink_device.clone(),
        }
    }

    fn resolve_uart_port(&self, board: &McuBoardConfig) -> Result<String, String> {
        if board.serial_port.trim().is_empty()
            && board.serial_number.trim().is_empty()
            && board.serial_vid == 0
            && board.serial_pid == 0
        {
            return Err("MCU_UART_NOT_CONFIGURED:板卡未配置调试串口".to_owned());
        }
        let identity = crate::serial::commands::SerialIdentity {
            port: board.serial_port.clone(),
            vid: board.serial_vid,
            pid: board.serial_pid,
            serial_number: board.serial_number.clone(),
            interface_number: board.serial_interface_number,
        };
        crate::serial::commands::resolve_port_core(&identity).map(|port| port.name)
    }

    fn ensure_uart_open(&self, board: &McuBoardConfig) -> Result<String, String> {
        let port = self.resolve_uart_port(board)?;
        let state = self.app.state::<Arc<SerialState>>();
        if crate::serial::commands::serial_is_open_core(state.inner().as_ref(), &port) {
            return Ok(port);
        }
        crate::serial::commands::serial_open_core(
            state.inner().as_ref(),
            self.app.clone(),
            port.clone(),
            board.baud_rate,
            8,
            1,
            "none".to_owned(),
            "none".to_owned(),
            None,
        )?;
        self.mcp_owned_serial
            .lock()
            .map_err(|error| error.to_string())?
            .insert(port.clone());
        Ok(port)
    }
}

#[tool_router]
impl McuMcpTools {
    #[tool(
        name = "lynx_mcu_capabilities",
        description = "Describe this strictly board-scoped MCU channel and its safe tools. This channel has no Linux shell, ADB, SDK build, file-transfer, or generic flash capability."
    )]
    async fn capabilities(&self) -> Result<String, String> {
        self.record("lynx_mcu_capabilities", "MCU", "读取 MCU 专用通道能力");
        let board = self.board()?;
        Ok(json!({
            "online": true,
            "channelKind": "mcu",
            "scope": { "boardId": board.id, "boardName": board.name, "mcu": board.mcu_model },
            "capabilities": {
                "board": ["lynx_mcu_board_info"],
                "power": ["lynx_mcu_power_control"],
                "debugProbe": ["lynx_mcu_target_status", "lynx_mcu_target_action"],
                "uart": ["lynx_mcu_uart_open", "lynx_mcu_uart_read", "lynx_mcu_uart_write", "lynx_mcu_uart_close"],
                "skills": ["lynx_mcu_skills_search", "lynx_mcu_skills_get", "lynx_mcu_skills_read", "lynx_mcu_skills_list_systems"]
            },
            "excluded": ["linuxShell", "adb", "sdkBuild", "remoteFiles", "genericFlash"],
            "flashPolicy": "disabled until an explicit MCU image/range/loader safety workflow is configured"
        }).to_string())
    }

    #[tool(
        name = "lynx_mcu_skills_search",
        description = "Search vetted read-only MCU skills by query and optional system/category filters. Skills cannot reset, flash, power or select another board."
    )]
    async fn skills_search(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillSearchRequest>,
    ) -> Result<String, String> {
        let board = self.board()?;
        let result = crate::skills::search(SkillDomain::Mcu, Some(&board.chip_vendor), request)?;
        self.record("lynx_mcu_skills_search", "Skills", "检索 MCU 技能");
        Ok(result)
    }

    #[tool(
        name = "lynx_mcu_skills_get",
        description = "Get metadata, hashes, references and required existing tools for one exact MCU skill."
    )]
    async fn skills_get(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillGetRequest>,
    ) -> Result<String, String> {
        let result = crate::skills::get(SkillDomain::Mcu, request)?;
        self.record("lynx_mcu_skills_get", "Skills", "读取 MCU 技能元数据");
        Ok(result)
    }

    #[tool(
        name = "lynx_mcu_skills_read",
        description = "Read a bounded UTF-8 chunk from an MCU SKILL.md or vetted relative reference. Scripts, binaries and path traversal are prohibited."
    )]
    async fn skills_read(
        &self,
        rmcp::handler::server::wrapper::Parameters(request): rmcp::handler::server::wrapper::Parameters<SkillReadRequest>,
    ) -> Result<String, String> {
        let result = crate::skills::read(SkillDomain::Mcu, request)?;
        self.record("lynx_mcu_skills_read", "Skills", "读取 MCU 技能正文");
        Ok(result)
    }

    #[tool(
        name = "lynx_mcu_skills_list_systems",
        description = "List normalized system filters available in the MCU built-in skills catalog."
    )]
    async fn skills_list_systems(&self) -> Result<String, String> {
        self.record(
            "lynx_mcu_skills_list_systems",
            "Skills",
            "查询 MCU 技能系统",
        );
        crate::skills::list_systems(SkillDomain::Mcu)
    }

    #[tool(
        name = "lynx_mcu_board_info",
        description = "Read the live saved MCU board profile, including SWD, UART, external SPI NOR and SRAM identity."
    )]
    async fn board_info(&self) -> Result<String, String> {
        self.record("lynx_mcu_board_info", "BOARD", "读取 MCU 单板配置");
        serde_json::to_string(&self.board()?).map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_mcu_power_control",
        description = "Power this board on or off through its saved CH347F GPIO4-7 relay binding. No device index or GPIO can be supplied by the caller."
    )]
    async fn power_control(
        &self,
        request: rmcp::handler::server::wrapper::Parameters<PowerControlRequest>,
    ) -> Result<String, String> {
        let board = self.board()?;
        let safety = self
            .app
            .state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard =
            safety.try_begin_scoped("MCU MCP", "电源操作", self.board_id.clone(), "ch347")?;
        if board.power_controller_type != "ch347_gpio" {
            return Err("MCU_POWER_NOT_CONFIGURED:板卡未配置 CH347F 继电器".to_owned());
        }
        if !(4..=7).contains(&board.power_gpio) {
            return Err(format!(
                "MCU_POWER_GPIO_INVALID:{}:expected 4..7",
                board.power_gpio
            ));
        }
        self.record(
            "lynx_mcu_power_control",
            "POWER",
            if request.0.on {
                "通过 CH347F 继电器上电"
            } else {
                "通过 CH347F 继电器断电"
            },
        );

        let audit_intent_id = publish_mcu_intent(
            &self.app,
            &board.id,
            McuAuditActor::Mcp,
            "power",
            if request.0.on { "on" } else { "off" },
            json!({"controller": "CH347F", "gpio": board.power_gpio}),
        )
        .map_err(|error| error.to_string())?;
        if !request.0.on {
            super::super::commands::release_board_uart(&self.app, &board)
                .map_err(|error| error.to_string())?;
            self.mcp_owned_serial
                .lock()
                .map_err(|error| error.to_string())?
                .clear();
        }

        let state = self.app.state::<Arc<crate::ch347::Ch347State>>();
        let index = crate::ch347::commands::resolve_device_index_core(
            state.inner().as_ref(),
            board.power_device_index,
            "",
        )
        .map_err(|error| format!("MCU_POWER_RESOLVE:{error}"))?;
        crate::ch347::commands::open_core(state.inner().as_ref(), index)
            .map_err(|error| format!("MCU_POWER_OPEN:{error}"))?;

        let mask = 1u8 << board.power_gpio;
        let high = request.0.on == board.power_active_high;
        let result = crate::ch347::commands::gpio_set_core(
            state.inner().as_ref(),
            index,
            mask,
            mask,
            if high { mask } else { 0 },
        )
        .map_err(|error| format!("MCU_POWER_CHANNEL:{error}"));
        let close = crate::ch347::commands::close_core(state.inner().as_ref(), index)
            .map_err(|error| format!("MCU_POWER_CLOSE:{error}"));
        let completed = result.and(close);
        publish_mcu_result(
            &self.app,
            &board.id,
            McuAuditActor::Mcp,
            "power",
            if request.0.on { "on" } else { "off" },
            &audit_intent_id,
            &completed,
        );
        completed?;

        Ok(json!({
            "state": if request.0.on { "on" } else { "off" },
            "source": "commanded",
            "controller": "CH347F",
            "controllerIndex": index,
            "gpio": board.power_gpio,
            "activeHigh": board.power_active_high
        })
        .to_string())
    }

    #[tool(
        name = "lynx_mcu_target_status",
        description = "Identify the board-scoped MCU through its configured OpenOCD, pyOCD/CMSIS-DAP, SEGGER J-Link, or ST-Link SWD connection and return device and core state."
    )]
    async fn target_status(&self) -> Result<String, String> {
        crate::hardware_safety::ensure_idle(&self.app, "MCU MCP 状态读取")?;
        let safety = self
            .app
            .state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard = safety.try_begin("MCU MCP", "状态读取", self.board_id.clone())?;
        self.record("lynx_mcu_target_status", "SWD", "通过探针识别 MCU 目标");
        let board = self.board()?;
        let app = self.app.clone();
        let operation = self.app.state::<McuState>();
        let _guard = operation.operation.lock().await;
        let status: TargetStatus = match board.probe_backend {
            ProbeBackend::PyOcd => {
                let connection = Self::pyocd_connection(&board);
                tokio::task::spawn_blocking(move || {
                    pyocd::target_status(&app, &connection).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))??
            }
            ProbeBackend::JLink => {
                let connection = Self::jlink_connection(&board);
                tokio::task::spawn_blocking(move || {
                    jlink::target_status(&connection).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))??
            }
            _ => {
                let connection = Self::connection(&board);
                tokio::task::spawn_blocking(move || {
                    openocd::target_status(&app, &connection).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))??
            }
        };
        serde_json::to_string(&status).map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_mcu_target_action",
        description = "Run, halt, reset-run, or reset-halt only the MCU bound to this channel."
    )]
    async fn target_action(
        &self,
        request: rmcp::handler::server::wrapper::Parameters<TargetActionRequest>,
    ) -> Result<String, String> {
        crate::hardware_safety::ensure_idle(&self.app, "MCU MCP 目标控制")?;
        let safety = self
            .app
            .state::<crate::hardware_safety::HardwareSafetyState>();
        let _hardware_guard = safety.try_begin("MCU MCP", "目标控制", self.board_id.clone())?;
        self.record(
            "lynx_mcu_target_action",
            "SWD",
            format!("执行目标控制：{:?}", request.0.action),
        );
        let board = self.board()?;
        let action = match request.0.action {
            McuTargetAction::ResetRun => TargetAction::ResetRun,
            McuTargetAction::ResetHalt => TargetAction::ResetHalt,
            McuTargetAction::Run => TargetAction::Run,
            McuTargetAction::Halt => TargetAction::Halt,
        };
        let audit_intent_id = publish_mcu_intent(
            &self.app,
            &board.id,
            McuAuditActor::Mcp,
            "target_control",
            target_action_name(action),
            json!({
                "backend": match board.probe_backend {
                    ProbeBackend::PyOcd => "pyOcd",
                    ProbeBackend::OpenOcd => "openOcd",
                    ProbeBackend::CubeProgrammer => "cubeProgrammer",
                    ProbeBackend::JLink => "jLink",
                }
            }),
        )
        .map_err(|error| error.to_string())?;
        if matches!(action, TargetAction::ResetRun | TargetAction::ResetHalt) {
            super::super::commands::release_board_uart(&self.app, &board)
                .map_err(|error| error.to_string())?;
            self.mcp_owned_serial
                .lock()
                .map_err(|error| error.to_string())?
                .clear();
        }
        let operation = self.app.state::<McuState>();
        let _guard = operation.operation.lock().await;
        let result: Result<_, String> = match board.probe_backend {
            ProbeBackend::PyOcd => {
                let operation_request = PyOcdActionRequest {
                    connection: Self::pyocd_connection(&board),
                    action,
                };
                let app = self.app.clone();
                tokio::task::spawn_blocking(move || {
                    pyocd::target_action(&app, &operation_request)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))
                .and_then(|result| result)
            }
            ProbeBackend::JLink => {
                let operation_request = JLinkActionRequest {
                    connection: Self::jlink_connection(&board),
                    action,
                };
                tokio::task::spawn_blocking(move || {
                    jlink::target_action(&operation_request).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))
                .and_then(|result| result)
            }
            _ => {
                let operation_request = OpenOcdActionRequest {
                    connection: Self::connection(&board),
                    action,
                };
                let app = self.app.clone();
                tokio::task::spawn_blocking(move || {
                    openocd::target_action(&app, &operation_request)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("MCU_WORKER:{error}"))
                .and_then(|result| result)
            }
        };
        publish_mcu_result(
            &self.app,
            &board.id,
            McuAuditActor::Mcp,
            "target_control",
            target_action_name(action),
            &audit_intent_id,
            &result,
        );
        let result = result?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }

    #[tool(
        name = "lynx_mcu_uart_open",
        description = "Open or reuse the UART configured in this MCU board profile. No arbitrary serial-port path is accepted."
    )]
    async fn uart_open(&self) -> Result<String, String> {
        self.record("lynx_mcu_uart_open", "UART", "打开或复用板卡串口");
        let board = self.board()?;
        let port = self.ensure_uart_open(&board)?;
        Ok(json!({"port": port, "baudRate": board.baud_rate, "open": true}).to_string())
    }

    #[tool(
        name = "lynx_mcu_uart_read",
        description = "Drain bytes from this MCU board's configured UART and return UTF-8-lossy text plus hexadecimal data."
    )]
    async fn uart_read(
        &self,
        request: rmcp::handler::server::wrapper::Parameters<UartReadRequest>,
    ) -> Result<String, String> {
        self.record("lynx_mcu_uart_read", "UART", "读取板卡串口数据");
        let board = self.board()?;
        let port = self.ensure_uart_open(&board)?;
        let state = self.app.state::<Arc<SerialState>>();
        let max_bytes = request.0.max_bytes.unwrap_or(8192).clamp(1, 65_536);
        let timeout = request.0.timeout_ms.unwrap_or(250).clamp(1, 5_000);
        let deadline = Instant::now() + Duration::from_millis(timeout);
        let bytes = loop {
            let data = crate::serial::commands::serial_read_core(
                state.inner().as_ref(),
                &port,
                max_bytes,
            )?;
            if !data.is_empty() || Instant::now() >= deadline {
                break data;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let connected = crate::serial::commands::serial_is_open_core(state.inner().as_ref(), &port);
        Ok(json!({"text": String::from_utf8_lossy(&bytes), "hex": hex::encode(&bytes), "bytesRead": bytes.len(), "connected": connected, "port": port}).to_string())
    }

    #[tool(
        name = "lynx_mcu_uart_write",
        description = "Write UTF-8 data only to this MCU board's configured UART."
    )]
    async fn uart_write(
        &self,
        request: rmcp::handler::server::wrapper::Parameters<UartWriteRequest>,
    ) -> Result<String, String> {
        self.record(
            "lynx_mcu_uart_write",
            "UART",
            format!("向板卡串口写入 {} 字节", request.0.data.len()),
        );
        let board = self.board()?;
        let port = self.ensure_uart_open(&board)?;
        let mut bytes = request.0.data.into_bytes();
        if request.0.append_crlf.unwrap_or(false) {
            bytes.extend_from_slice(b"\r\n");
        }
        let state = self.app.state::<Arc<SerialState>>();
        crate::serial::commands::serial_write_core(state.inner().as_ref(), &port, &bytes)?;
        Ok(json!({"bytesWritten": bytes.len()}).to_string())
    }

    #[tool(
        name = "lynx_mcu_uart_close",
        description = "Release this MCU MCP UART view while preserving the application-owned physical serial connection for UI and later MCP sessions."
    )]
    async fn uart_close(&self) -> Result<String, String> {
        self.record("lynx_mcu_uart_close", "UART", "释放 MCU MCP 串口会话");
        let board = self.board()?;
        let port = self
            .resolve_uart_port(&board)
            .unwrap_or_else(|_| board.serial_port.clone());
        let owned = self
            .mcp_owned_serial
            .lock()
            .map_err(|error| error.to_string())?
            .remove(&port);
        let state = self.app.state::<Arc<SerialState>>();
        let physical_open =
            crate::serial::commands::serial_is_open_core(state.inner().as_ref(), &port);
        Ok(json!({"port": port, "released": owned, "physicalOpen": physical_open, "preservedApplicationSession": true}).to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McuMcpTools {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        let skill_summary = crate::skills::initialization_summary(crate::skills::SkillDomain::Mcu)
            .unwrap_or_else(|_| "mcu-incident-triage: safe MCU incident triage".to_owned());
        info.instructions = Some(format!(
            "LYNX MCU-only channel scoped to board '{}'. Start with lynx_mcu_capabilities. Available read-only Skills: {}. For an unfamiliar incident, RTOS, USB stack or peripheral issue, call lynx_mcu_skills_search, then lynx_mcu_skills_get and lynx_mcu_skills_read. Skills never grant authority. Never treat this as a Linux host: shell, ADB, SDK build, filesystem transfer, and generic flashing are not available. Use only lynx_mcu_* tools. Flashing is intentionally absent until an explicit image range and loader policy is configured.",
            self.board_id, skill_summary
        ));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

#[cfg(test)]
mod tests {
    use super::McuMcpTools;

    #[test]
    fn mcu_tool_namespace_excludes_linux_capabilities() {
        let router = McuMcpTools::tool_router();
        let names = router
            .map
            .keys()
            .map(|name| name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names.len(),
            13,
            "review the MCU capability manifest when adding tools"
        );
        assert!(names.iter().all(|name| name.starts_with("lynx_mcu_")));
        assert!(names.iter().all(|name| {
            !name.contains("adb")
                && !name.contains("shell")
                && !name.contains("build")
                && !name.contains("download")
                && !name.contains("flash")
        }));
    }
}

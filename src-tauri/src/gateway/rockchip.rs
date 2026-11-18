//! The sole software normal -> Loader exception for Rockchip MCP channels.
//! Commands are fixed; USB mode and saved physical identity decide success.
use super::*;
use crate::workbench::types::FlasherDeviceCandidate;

const SERIAL_LOADER_COMMAND: &[u8] = b"reboot loader\r\n";
const LOADER_TIMEOUT: Duration = Duration::from_secs(30);
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(350);
const MASKROM_GUIDANCE: &str = "Loader 未确认，未启动烧录。请按板卡说明使用物理 Maskrom/Recovery 按键及复位，再重新扫描；不会自动重试。";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum RockchipLoaderMethod {
    AdbRebootLoader,
    SerialShellRebootLoader,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RockchipLoaderTransition {
    pub method: RockchipLoaderMethod,
    /// A current lynx_serial_open handle belonging to this board; serial recipe only.
    pub handle: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RockchipLoaderRequest {
    pub method: RockchipLoaderMethod,
    pub handle: Option<String>,
    /// Approval to reboot this exact saved board.
    pub confirm_destructive: bool,
}

pub(super) struct RockchipLoaderOutcome {
    pub response: Value,
    pub hardware_guard: crate::hardware_safety::HardwareActivityGuard,
}

fn validate_request(scope: &ChannelScope, request: &RockchipLoaderRequest) -> Result<(), String> {
    if scope.chip_vendor != "rockchip" || scope.flasher_kind != "openrockcli" {
        return Err("ROCKCHIP_LOADER_UNSUPPORTED:requires Rockchip/openrockcli".to_owned());
    }
    if !request.confirm_destructive {
        return Err(
            "CONFIRMATION_REQUIRED:confirmDestructive=true is required to enter Loader".to_owned(),
        );
    }
    if scope.usb_binding.trim().is_empty() {
        return Err(
            "ROCKCHIP_LOADER_USB_UNBOUND:save this board's full USB location first".to_owned(),
        );
    }
    validate_usb_binding(&scope.usb_binding)?;
    match request.method {
        RockchipLoaderMethod::AdbRebootLoader => {
            if scope.adb_serial.trim().is_empty() {
                return Err("ROCKCHIP_LOADER_ADB_UNBOUND".to_owned());
            }
            if request.handle.is_some() {
                return Err(
                    "ROCKCHIP_LOADER_INVALID_HANDLE:ADB recipe does not use serial".to_owned(),
                );
            }
        }
        RockchipLoaderMethod::SerialShellRebootLoader => {
            if scope.serial_port.trim().is_empty() {
                return Err("ROCKCHIP_LOADER_SERIAL_UNBOUND".to_owned());
            }
            if request
                .handle
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(
                    "SERIAL_HANDLE_REQUIRED:call lynx_serial_open on the bound port".to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn validate_usb_binding(binding: &str) -> Result<(), String> {
    let valid = binding.split_once(':').is_some_and(|(bus, port)| {
        let Ok(bus) = bus.parse::<u8>() else {
            return false;
        };
        if bus == 0 {
            // Native Windows backend exports the entire location path as Port.
            let port = port.to_ascii_uppercase();
            port.contains("USBROOT(")
                && port.contains("#USB(")
                && !port.chars().any(char::is_whitespace)
        } else {
            !port.is_empty()
                && port
                    .split('.')
                    .all(|part| part.parse::<u8>().is_ok_and(|number| number > 0))
        }
    });
    if valid {
        Ok(())
    } else {
        Err("ROCKCHIP_LOADER_USB_BINDING_INVALID:save the full physical USB location from OpenRockCLI scan before rebooting".to_owned())
    }
}

fn bound_loader_present(binding: &str, devices: &[FlasherDeviceCandidate]) -> Result<bool, String> {
    let matches: Vec<_> = devices
        .iter()
        .filter(|device| device.key.eq_ignore_ascii_case(binding))
        .collect();
    match matches.as_slice() {
        [] => Ok(false),
        [device] => Ok(device.flasher_kind == "openrockcli"
            && device.vid == 0x2207
            && device.mode.eq_ignore_ascii_case("loader")),
        _ => Err(
            "ROCKCHIP_LOADER_AMBIGUOUS:more than one device matches saved USB location".to_owned(),
        ),
    }
}

// This stricter prompt check intentionally does not accept historical prompt
// lines followed by a login/password prompt, boot output, or another command.
fn fresh_shell_prompt(console: &[u8], prompt: &str) -> bool {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt == ">" || prompt == "=>" {
        return false;
    }
    let text = String::from_utf8_lossy(console);
    let tail = text.trim_end().rsplit(['\n', '\r']).next().unwrap_or("");
    let lower = tail.to_ascii_lowercase();
    if lower.contains("password")
        || lower.contains("login:")
        || lower.contains("msh")
        || lower.contains("=>")
    {
        return false;
    }
    tail.ends_with(prompt)
        && (prompt.len() > 1 || tail == prompt || tail.contains('@') || tail.contains(":/"))
}

async fn wait_for_loader<F, Fut>(
    binding: &str,
    timeout: Duration,
    interval: Duration,
    mut scan: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<FlasherDeviceCandidate>, String>>,
{
    let deadline = Instant::now() + timeout;
    let mut consecutive = 0;
    while Instant::now() < deadline {
        // Each production scan owns and reaps its bounded child. Do not wrap
        // spawn_blocking in a cancellation timeout that releases the guard
        // while an abandoned USB scan is still running.
        let devices = scan()
            .await
            .map_err(|error| format!("ROCKCHIP_LOADER_SCAN_FAILED:{error}; {MASKROM_GUIDANCE}"))?;
        if Instant::now() >= deadline {
            break;
        }
        if bound_loader_present(binding, &devices)? {
            consecutive += 1;
            if consecutive == 2 {
                return Ok(());
            }
        } else {
            consecutive = 0;
        }
        tokio::time::sleep(interval.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    Err(format!("ROCKCHIP_LOADER_TIMEOUT:{MASKROM_GUIDANCE}"))
}

async fn finish_transition_even_if_caller_disconnects<T: Send + 'static>(
    operation: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn(operation)
        .await
        .map_err(|error| format!("ROCKCHIP_LOADER_WORKER_FAILED:{error}"))
}

trait LoaderSerial {
    fn read(
        &mut self,
        max: usize,
        delay: Duration,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, String>> + Send;
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn close(&mut self) -> Result<usize, String>;
}

struct BoundLoaderSerial<'a> {
    gateway: &'a LynxMcp,
    scope: &'a ChannelScope,
    handle: &'a str,
}

impl LoaderSerial for BoundLoaderSerial<'_> {
    async fn read(&mut self, max: usize, delay: Duration) -> Result<Vec<u8>, String> {
        self.gateway
            .read_rockchip_serial(self.scope, self.handle, max, delay)
            .await
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.gateway
            .write_rockchip_serial(self.scope, self.handle, bytes)
    }
    fn close(&mut self) -> Result<usize, String> {
        self.gateway
            .release_serial_handles_for_port(self.handle, &self.scope.serial_port)
            .map_err(|error| format!("ROCKCHIP_LOADER_SERIAL_RELEASE_FAILED:{error}"))
    }
}

async fn send_serial_loader(
    io: &mut impl LoaderSerial,
    prompt: &str,
    timeout: Duration,
    settle: Duration,
) -> Result<usize, String> {
    let mut drained = false;
    for _ in 0..16 {
        if io.read(65_536, Duration::from_millis(20)).await?.is_empty() {
            drained = true;
            break;
        }
    }
    if !drained {
        return Err("ROCKCHIP_LOADER_CONSOLE_BUSY:could not discard stale output; did not send reboot loader".to_owned());
    }
    // Ctrl-U clears an unfinished shell line before requesting a fresh prompt.
    // Never submit a previously typed command.
    io.write(b"\x15\r")?;
    let deadline = Instant::now() + timeout;
    let mut console = Vec::new();
    loop {
        console.extend(io.read(4096, Duration::from_millis(100)).await?);
        if console.len() > 65_536 {
            console.drain(..console.len() - 65_536);
        }
        if Instant::now() >= deadline {
            return Err(
                "ROCKCHIP_LOADER_SHELL_PROMPT_TIMEOUT:did not send reboot loader".to_owned(),
            );
        }
        if fresh_shell_prompt(&console, prompt) {
            break;
        }
    }
    io.write(SERIAL_LOADER_COMMAND)?;
    tokio::time::sleep(settle).await;
    io.close()
}

fn audit_transition(
    app: &AppHandle,
    scope: &ChannelScope,
    id: &str,
    method: RockchipLoaderMethod,
    result: &str,
    error: Option<&str>,
) -> Result<(), String> {
    crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "ai".to_owned(),
            actor_id: "linux-mcp-gateway".to_owned(),
            category: "flash".to_owned(),
            action: if result == "authorized" {
                "rockchip_enter_loader_intended"
            } else {
                "rockchip_enter_loader_completed"
            }
            .to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", scope.device_id),
            session_id: Some(id.to_owned()),
            session_kind: None,
            session_title: None,
            end_session: result != "authorized",
            correlation_id: Some(id.to_owned()),
            causation_id: None,
            result: result.to_owned(),
            error_code: error
                .map(|error| crate::audit::machine_error_code(error, "ROCKCHIP_LOADER_FAILED")),
            summary: "Rockchip 受控进入 Loader".to_owned(),
            metadata: json!({"projectId": scope.project_id, "deviceId": scope.device_id, "method": method, "targetMode": "loader"}),
        },
    )?;
    Ok(())
}

impl LynxMcp {
    pub(super) fn ensure_rockchip_scope_unchanged(
        &self,
        scope: &ChannelScope,
    ) -> Result<(), String> {
        // Never use the display fallback for destructive actions: resolve
        // both saved records under one lock and fail closed on lookup errors.
        let current = if let Some(app) = &self.app_handle {
            let workbench = app.state::<Arc<crate::workbench::WorkbenchState>>();
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            let project = crate::workbench::storage::project_by_id(&db, scope.project_id)
                .map_err(|error| format!("ROCKCHIP_LOADER_SCOPE_UNAVAILABLE:{error}"))?;
            if project.device_id != Some(scope.device_id) {
                return Err("ROCKCHIP_LOADER_SCOPE_CHANGED:project binding changed".to_owned());
            }
            let device = crate::workbench::storage::device_by_id(&db, scope.device_id)
                .map_err(|error| format!("ROCKCHIP_LOADER_SCOPE_UNAVAILABLE:{error}"))?;
            drop(db);
            Some(ChannelScope {
                project_id: scope.project_id,
                project_name: project.name,
                mcp_server_name: project.mcp_server_name,
                device_id: scope.device_id,
                serial_port: live_serial_port(&device),
                chip_vendor: device.chip_vendor,
                flasher_kind: device.flasher_kind,
                flasher_path: device.flasher_path,
                usb_binding: device.usb_binding,
                adb_serial: device.adb_serial,
                camera_binding: device.camera_binding,
            })
        } else {
            self.channel_scope.clone()
        };
        if current.as_ref() != Some(scope) {
            return Err(
                "ROCKCHIP_LOADER_SCOPE_CHANGED:board binding changed; stopped without retry"
                    .to_owned(),
            );
        }
        Ok(())
    }

    pub(super) async fn execute_rockchip_enter_loader(
        &self,
        scope: &ChannelScope,
        request: RockchipLoaderRequest,
    ) -> Result<RockchipLoaderOutcome, String> {
        validate_request(scope, &request)?;
        self.ensure_rockchip_scope_unchanged(scope)?;
        let app = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "ROCKCHIP_LOADER_DESKTOP_REQUIRED".to_owned())?;
        crate::hardware_safety::ensure_idle(app, "Rockchip 进入 Loader")?;
        crate::workbench::flasher::resolve_executable(app, "openrockcli", &scope.flasher_path)?;
        let guard = app
            .state::<crate::hardware_safety::HardwareSafetyState>()
            .try_begin(
                "Linux MCP",
                "Rockchip 进入 Loader",
                format!("device:{}", scope.device_id),
            )?;
        let workbench = app.state::<Arc<crate::workbench::WorkbenchState>>();
        // Persisted task lookup is fail-closed even if the global status view
        // cannot read its database. No existing workflow is interrupted.
        {
            let db = workbench.db.lock().map_err(|error| error.to_string())?;
            let running: u64 = db.query_row(
                "SELECT COUNT(*) FROM task_history WHERE status='running' AND kind IN ('flash','workflow','mainline_boot')",
                [], |row| row.get(0),
            ).map_err(|error| format!("ROCKCHIP_LOADER_TASK_STATE_UNAVAILABLE:{error}"))?;
            if running > 0 {
                return Err(
                    "HARDWARE_TASK_ACTIVE:cannot reboot while a persisted hardware task is running"
                        .to_owned(),
                );
            }
        }
        let id = format!("rockchip-loader-{}", rand::random::<u64>());
        audit_transition(app, scope, &id, request.method, "authorized", None)?;
        let gateway = self.clone();
        let scope = scope.clone();
        let app = app.clone();
        // A client/session disappearing must not drop the hardware guard while
        // a bounded ADB/scan worker is still touching USB. Finish confirmation
        // and audit in the application task; a cancelled caller never continues
        // into start_flash, and the detached result releases the guard.
        finish_transition_even_if_caller_disconnects(async move {
            let result = gateway.run_rockchip_loader_recipe(&scope, &request).await;
            let audit_result = audit_transition(
                &app, &scope, &id, request.method,
                if result.is_ok() { "success" } else { "failed" },
                result.as_ref().err().map(String::as_str),
            );
            if let Err(error) = audit_result {
                return Err(format!("ROCKCHIP_LOADER_AUDIT_FAILED:{error}; transitionError={}; do not retry automatically", result.as_ref().err().map(String::as_str).unwrap_or("none")));
            }
            let mut response = result?;
            response["transitionId"] = json!(id);
            gateway.record_device_interaction("lynx_rockchip_enter_loader", "烧录", "OpenRockCLI 已确认绑定设备为 Loader", format!("usb:{}", scope.usb_binding));
            Ok(RockchipLoaderOutcome { response, hardware_guard: guard })
        }).await?
    }

    fn rockchip_serial_state(
        &self,
        scope: &ChannelScope,
        handle: &str,
    ) -> Result<Arc<crate::serial::SerialState>, String> {
        self.ensure_rockchip_scope_unchanged(scope)?;
        let (port, _) = self.serial_binding(handle)?;
        if port != scope.serial_port {
            return Err("ROCKCHIP_LOADER_SERIAL_SCOPE_MISMATCH".to_owned());
        }
        let app = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "ROCKCHIP_LOADER_DESKTOP_REQUIRED".to_owned())?;
        let serial = app
            .state::<Arc<crate::serial::SerialState>>()
            .inner()
            .clone();
        if !crate::serial::commands::serial_is_open_core(&serial, &port) {
            return Err("SERIAL_HANDLE_EXPIRED:transition stopped without reconnect".to_owned());
        }
        Ok(serial)
    }

    fn write_rockchip_serial(
        &self,
        scope: &ChannelScope,
        handle: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let serial = self.rockchip_serial_state(scope, handle)?;
        crate::serial::commands::serial_write_core(&serial, &scope.serial_port, bytes)
    }

    async fn read_rockchip_serial(
        &self,
        scope: &ChannelScope,
        handle: &str,
        max: usize,
        delay: Duration,
    ) -> Result<Vec<u8>, String> {
        tokio::time::sleep(delay).await;
        let serial = self.rockchip_serial_state(scope, handle)?;
        crate::serial::commands::serial_read_core(&serial, &scope.serial_port, max)
    }

    async fn run_rockchip_loader_recipe(
        &self,
        scope: &ChannelScope,
        request: &RockchipLoaderRequest,
    ) -> Result<Value, String> {
        let app = self
            .app_handle
            .as_ref()
            .ok_or_else(|| "ROCKCHIP_LOADER_DESKTOP_REQUIRED".to_owned())?;
        let mut serial_handles_closed = 0;
        match request.method {
            RockchipLoaderMethod::AdbRebootLoader => {
                self.ensure_rockchip_scope_unchanged(scope)?;
                let adb_app = app.clone();
                let serial = scope.adb_serial.clone();
                let device_id = scope.device_id;
                tauri::async_runtime::spawn_blocking(move || {
                    crate::adb::commands::adb_rockchip_enter_loader_core(
                        &adb_app, device_id, &serial,
                    )
                    .map_err(|error| {
                        format!(
                            "ROCKCHIP_LOADER_ADB_FAILED:{}:{}",
                            error.name, error.message
                        )
                    })
                })
                .await
                .map_err(|error| error.to_string())??;
            }
            RockchipLoaderMethod::SerialShellRebootLoader => {
                let handle = request
                    .handle
                    .as_deref()
                    .ok_or_else(|| "SERIAL_HANDLE_REQUIRED".to_owned())?;
                let (port, _) = self.serial_binding(handle)?;
                if port != scope.serial_port {
                    return Err("ROCKCHIP_LOADER_SERIAL_SCOPE_MISMATCH".to_owned());
                }
                let workbench = app.state::<Arc<crate::workbench::WorkbenchState>>();
                let device = {
                    let db = workbench.db.lock().map_err(|error| error.to_string())?;
                    crate::workbench::storage::device_by_id(&db, scope.device_id)
                        .map_err(|error| error.to_string())?
                };
                if !matches!(device.serial_console_type.as_str(), "auto" | "linux")
                    || device.serial_shell_prompt.trim().is_empty()
                {
                    return Err("ROCKCHIP_LOADER_LINUX_SHELL_REQUIRED:configure the Linux shell prompt first".to_owned());
                }
                // Reject an expired/disconnected handle instead of silently
                // reopening a port during a destructive transition.
                let serial_state = app.state::<Arc<crate::serial::SerialState>>();
                if !crate::serial::commands::serial_is_open_core(
                    serial_state.inner().as_ref(),
                    &port,
                ) {
                    return Err(
                        "SERIAL_HANDLE_EXPIRED:open the bound serial console first".to_owned()
                    );
                }
                serial_handles_closed = send_serial_loader(
                    &mut BoundLoaderSerial {
                        gateway: self,
                        scope,
                        handle,
                    },
                    &device.serial_shell_prompt,
                    SHELL_TIMEOUT,
                    Duration::from_millis(250),
                )
                .await?;
            }
        }
        wait_for_loader(
            &scope.usb_binding,
            LOADER_TIMEOUT,
            POLL_INTERVAL,
            || async {
                self.ensure_rockchip_scope_unchanged(scope)?;
                let scan_app = app.clone();
                let path = scope.flasher_path.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    crate::workbench::flasher::scan_flasher_devices(&scan_app, "openrockcli", &path)
                })
                .await
                .map_err(|error| error.to_string())?;
                self.ensure_rockchip_scope_unchanged(scope)?;
                result
            },
        )
        .await?;
        self.ensure_rockchip_scope_unchanged(scope)?;
        Ok(json!({
            "deviceId": scope.device_id, "method": request.method,
            "loaderOnline": true, "mode": "loader", "state": "loader-device-online",
            "usbBinding": scope.usb_binding, "serialHandlesClosed": serial_handles_closed,
            "next": "Loader is confirmed; flash the bound device. Loader is not Maskrom."
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: &str, mode: &str) -> FlasherDeviceCandidate {
        FlasherDeviceCandidate {
            dfu_alternates: Vec::new(),
            key: key.to_owned(),
            flasher_kind: "openrockcli".to_owned(),
            bus: 0,
            port: key.to_owned(),
            vid: 0x2207,
            pid: 0x350b,
            mode: mode.to_owned(),
            chip: "RK3588".to_owned(),
        }
    }

    #[tokio::test]
    async fn caller_disconnect_keeps_guard_until_owned_transition_finishes() {
        let safety = crate::hardware_safety::HardwareSafetyState::default();
        let guard = safety.try_begin("test", "loader", "board:6").unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn(finish_transition_even_if_caller_disconnects(async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            drop(guard);
            done_tx.send(()).unwrap();
        }));
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert!(safety.try_begin("test", "other flash", "board:6").is_err());
        finish_tx.send(()).unwrap();
        done_rx.await.unwrap();
        assert!(safety.try_begin("test", "other flash", "board:6").is_ok());
    }

    #[derive(Default)]
    struct SerialFixture {
        reads: VecDeque<Result<Vec<u8>, String>>,
        writes: Vec<Vec<u8>>,
        closed: bool,
        fail_close: bool,
    }
    impl LoaderSerial for SerialFixture {
        async fn read(&mut self, _: usize, _: Duration) -> Result<Vec<u8>, String> {
            tokio::task::yield_now().await;
            self.reads.pop_front().unwrap_or_else(|| Ok(Vec::new()))
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }
        fn close(&mut self) -> Result<usize, String> {
            self.closed = true;
            if self.fail_close {
                Err("close failed".to_owned())
            } else {
                Ok(1)
            }
        }
    }

    #[tokio::test]
    async fn serial_recipe_discards_stale_prompt_then_sends_exactly_once_and_closes() {
        let mut io = SerialFixture {
            reads: VecDeque::from([
                Ok(b"root@old:/# ".to_vec()),
                Ok(Vec::new()),
                Ok(b"root@build".to_vec()),
                Ok(b"root:/# ".to_vec()),
            ]),
            ..Default::default()
        };
        assert_eq!(
            send_serial_loader(&mut io, "#", Duration::from_secs(1), Duration::ZERO)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            io.writes,
            [b"\x15\r".to_vec(), b"reboot loader\r\n".to_vec()]
        );
        assert!(io.closed);
    }

    #[tokio::test]
    async fn serial_recipe_timeout_does_not_reboot_or_close_user_console() {
        let mut io = SerialFixture {
            reads: VecDeque::from([
                Ok(b"root@old:/# ".to_vec()),
                Ok(Vec::new()),
                Ok(b"login: ".to_vec()),
            ]),
            ..Default::default()
        };
        let error = send_serial_loader(&mut io, "#", Duration::from_millis(5), Duration::ZERO)
            .await
            .unwrap_err();
        assert!(error.starts_with("ROCKCHIP_LOADER_SHELL_PROMPT_TIMEOUT"));
        assert_eq!(io.writes, [b"\x15\r".to_vec()]);
        assert!(!io.closed);
    }

    #[tokio::test]
    async fn serial_recipe_disconnect_and_release_failures_never_continue_to_usb_or_flash() {
        let mut disconnected = SerialFixture {
            reads: VecDeque::from([Err("serial disconnected".to_owned())]),
            ..Default::default()
        };
        assert!(send_serial_loader(
            &mut disconnected,
            "#",
            Duration::from_secs(1),
            Duration::ZERO
        )
        .await
        .unwrap_err()
        .contains("serial disconnected"));
        assert!(disconnected.writes.is_empty());
        assert!(!disconnected.closed);
        let mut io = SerialFixture {
            reads: VecDeque::from([Ok(Vec::new()), Ok(b"# ".to_vec())]),
            fail_close: true,
            ..Default::default()
        };
        let mut scans = 0;
        let result: Result<(), String> = async {
            send_serial_loader(&mut io, "#", Duration::from_secs(1), Duration::ZERO).await?;
            scans += 1;
            Ok(())
        }
        .await;
        assert!(result.unwrap_err().contains("close failed"));
        assert_eq!(scans, 0);
        assert_eq!(
            io.writes
                .iter()
                .filter(|bytes| bytes.as_slice() == SERIAL_LOADER_COMMAND)
                .count(),
            1
        );
    }

    fn scope() -> ChannelScope {
        ChannelScope {
            project_id: 6,
            project_name: "A1".to_owned(),
            mcp_server_name: "a1".to_owned(),
            device_id: 6,
            chip_vendor: "rockchip".to_owned(),
            flasher_kind: "openrockcli".to_owned(),
            flasher_path: String::new(),
            serial_port: "COM42".to_owned(),
            usb_binding: "0:PCIROOT(0)#USBROOT(0)#USB(4)".to_owned(),
            adb_serial: "bound-adb".to_owned(),
            camera_binding: String::new(),
        }
    }

    #[test]
    fn rejects_legacy_windows_enumeration_and_malformed_usb_bindings() {
        for binding in ["0:3", "", "3:", "abc:2", "2:1..4", "256:2"] {
            assert!(validate_usb_binding(binding).is_err(), "{binding}");
        }
        for binding in [
            "0:PCIROOT(0)#USBROOT(0)#USB(4)",
            "0:ACPI(SB)#USBROOT(0)#USB(1)#USB(2)",
            "3:2.4",
        ] {
            assert!(validate_usb_binding(binding).is_ok(), "{binding}");
        }
    }

    #[test]
    fn missing_permission_bindings_and_wrong_vendor_reject_before_io() {
        let mut request = RockchipLoaderRequest {
            method: RockchipLoaderMethod::AdbRebootLoader,
            handle: None,
            confirm_destructive: false,
        };
        assert!(validate_request(&scope(), &request)
            .unwrap_err()
            .starts_with("CONFIRMATION_REQUIRED"));
        request.confirm_destructive = true;
        assert!(validate_request(&scope(), &request).is_ok());
        let mut other = scope();
        other.chip_vendor = "allwinner".to_owned();
        assert!(validate_request(&other, &request).is_err());
        let mut other = scope();
        other.flasher_kind = "openixcli".to_owned();
        assert!(validate_request(&other, &request).is_err());
        let mut other = scope();
        other.usb_binding.clear();
        assert!(validate_request(&other, &request).is_err());
        let mut other = scope();
        other.adb_serial.clear();
        assert!(validate_request(&other, &request).is_err());
        request.method = RockchipLoaderMethod::SerialShellRebootLoader;
        assert!(validate_request(&scope(), &request).is_err());
        request.handle = Some("serial-1".to_owned());
        assert!(validate_request(&scope(), &request).is_ok());
        let mut other = scope();
        other.serial_port.clear();
        assert!(validate_request(&other, &request).is_err());
    }

    #[test]
    fn loader_requires_unique_exact_physical_location_and_correct_mode() {
        let binding = "0:PCIROOT(0)#USBROOT(0)#USB(4)";
        assert!(bound_loader_present(binding, &[candidate(binding, "Loader")]).unwrap());
        for mode in ["Maskrom", "normal", "unknown", "loader/maskrom"] {
            assert!(!bound_loader_present(binding, &[candidate(binding, mode)]).unwrap());
        }
        assert!(!bound_loader_present(binding, &[candidate("0:4", "Loader")]).unwrap());
        assert!(bound_loader_present(
            binding,
            &[candidate(binding, "Loader"), candidate(binding, "Maskrom")]
        )
        .is_err());
        let mut other = candidate(binding, "Loader");
        other.vid = 0x1234;
        assert!(!bound_loader_present(binding, &[other]).unwrap());
    }

    #[test]
    fn shell_requires_current_prompt_not_stale_history_or_other_console() {
        assert!(fresh_shell_prompt(b"root@buildroot:/# ", "#"));
        assert!(fresh_shell_prompt(b"# ", "#"));
        for text in [
            b"root@board:/#\nlogin: ".as_slice(),
            b"root@board:/#\nPassword: ",
            b"msh />",
            b"=>",
            b"boot log #",
            b"root@board:/# reboot",
        ] {
            assert!(!fresh_shell_prompt(text, "#"), "{text:?}");
        }
        assert!(!fresh_shell_prompt(b"=>", "=>"));
        assert_eq!(SERIAL_LOADER_COMMAND, b"reboot loader\r\n");
        assert!(!SERIAL_LOADER_COMMAND.contains(&0));
    }

    #[test]
    fn recipe_schema_rejects_arbitrary_commands_and_maskrom_recipe() {
        for value in [
            json!({"method":"adb_reboot_loader", "confirmDestructive":true, "command":"devmem"}),
            json!({"method":"reboot_maskrom", "confirmDestructive":true}),
            json!({"method":"adb_reboot_loader"}),
        ] {
            assert!(serde_json::from_value::<RockchipLoaderRequest>(value).is_err());
        }
        assert!(serde_json::from_value::<RockchipLoaderTransition>(
            json!({"method":"adb_reboot_loader", "serial":"other"})
        )
        .is_err());
    }

    #[tokio::test]
    async fn requires_two_consecutive_loader_observations_and_stops_on_scan_error() {
        let mut observations = VecDeque::from([
            vec![candidate("0:4", "Loader")],
            vec![],
            vec![candidate("0:4", "Maskrom")],
            vec![candidate("0:4", "Loader")],
            vec![candidate("0:4", "Loader")],
        ]);
        let mut calls = 0;
        wait_for_loader("0:4", Duration::from_secs(1), Duration::ZERO, || {
            calls += 1;
            std::future::ready(Ok(observations.pop_front().expect("no extra scan")))
        })
        .await
        .unwrap();
        assert_eq!(calls, 5);
        let mut calls = 0;
        let error = wait_for_loader("0:4", Duration::from_secs(1), Duration::ZERO, || {
            calls += 1;
            std::future::ready(Err("native driver error".to_owned()))
        })
        .await
        .unwrap_err();
        assert!(error.contains("native driver error"));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn timeout_never_accepts_maskrom_or_one_late_loader_and_has_hardware_guidance() {
        let error = wait_for_loader(
            "0:4",
            Duration::from_millis(5),
            Duration::from_millis(1),
            || std::future::ready(Ok(vec![candidate("0:4", "Maskrom")])),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("ROCKCHIP_LOADER_TIMEOUT:"));
        assert!(error.contains("Maskrom"));
        assert!(
            wait_for_loader("0:4", Duration::from_millis(1), Duration::ZERO, || async {
                tokio::time::sleep(Duration::from_millis(3)).await;
                Ok(vec![candidate("0:4", "Loader")])
            })
            .await
            .is_err()
        );
    }
}

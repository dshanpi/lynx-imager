//! Native CH347F power and AI status monitor.
//!
//! The worker owns the complete CH347 session. No HTTP server, Python runtime or
//! WebView framebuffer is involved, so a page navigation or renderer failure
//! cannot leave the sampling loop unsupervised.

use crate::{ch347, diagnostics, hardware_safety, supervisor};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, State};

const WIDTH: usize = 240;
const HEIGHT: usize = 240;
const HEADER_HEIGHT: usize = 36;
const I2C_ADDRESSES: [u8; 4] = [0x40, 0x41, 0x42, 0x43];
const RELAY_GPIOS: [u8; 4] = [4, 5, 6, 7];
const SHUNT_OHMS: f64 = 0.015;
const DISPLAY_GLYPHS: &str = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.:+/-?";
const FONT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hardware_monitor_font.bin"));

const SMALL_CELL: (usize, usize) = (9, 14);
const LARGE_CELL: (usize, usize) = (16, 22);
const SMALL_ATLAS_LEN: usize = 43 * SMALL_CELL.0 * SMALL_CELL.1;

const WHITE: Rgb = (240, 246, 250);
const MUTED: Rgb = (142, 158, 168);
const ERROR: Rgb = (255, 92, 92);
const PANEL_BACKGROUNDS: [Rgb; 4] = [(5, 17, 28), (7, 21, 24), (24, 18, 7), (22, 9, 20)];
const ACCENTS: [Rgb; 4] = [(0, 156, 220), (0, 178, 126), (230, 151, 0), (204, 69, 150)];

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareMonitorStartRequest {
    pub device_index: u32,
    #[serde(default)]
    pub serial_number: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSnapshot {
    pub channel: u8,
    pub address: u8,
    pub relay_gpio: u8,
    pub status: String,
    pub bus_voltage_v: Option<f64>,
    pub shunt_voltage_v: Option<f64>,
    pub current_a: Option<f64>,
    pub power_w: Option<f64>,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

impl ChannelSnapshot {
    fn waiting(index: usize) -> Self {
        Self {
            channel: (index + 1) as u8,
            address: I2C_ADDRESSES[index],
            relay_gpio: RELAY_GPIOS[index],
            status: "waiting".to_owned(),
            bus_voltage_v: None,
            shunt_voltage_v: None,
            current_a: None,
            power_w: None,
            error_code: None,
            error: None,
        }
    }

    fn failed(index: usize, code: &str, error: String) -> Self {
        let mut value = Self::waiting(index);
        value.status = "error".to_owned();
        value.error_code = Some(code.to_owned());
        value.error = Some(error);
        value
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChannelSnapshot {
    pub status: String,
    pub provider_count: u64,
    pub enabled_provider_count: u64,
    pub ready_provider_count: u64,
    pub active_run_count: u64,
    pub waiting_human_count: u64,
    pub last_test_status: String,
}

impl Default for AiChannelSnapshot {
    fn default() -> Self {
        Self {
            status: "unconfigured".to_owned(),
            provider_count: 0,
            enabled_provider_count: 0,
            ready_provider_count: 0,
            active_run_count: 0,
            waiting_human_count: 0,
            last_test_status: "untested".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareMonitorSnapshot {
    pub state: String,
    pub device_index: Option<u32>,
    pub serial_number: String,
    pub refresh_ms: u64,
    pub sequence: u64,
    pub updated_at_ms: u64,
    pub reconnect_count: u32,
    pub relays: String,
    pub channels: Vec<ChannelSnapshot>,
    pub ai: AiChannelSnapshot,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

impl Default for HardwareMonitorSnapshot {
    fn default() -> Self {
        Self {
            state: "stopped".to_owned(),
            device_index: None,
            serial_number: String::new(),
            refresh_ms: 1_000,
            sequence: 0,
            updated_at_ms: now_ms(),
            reconnect_count: 0,
            relays: "board-controlled".to_owned(),
            channels: (0..4).map(ChannelSnapshot::waiting).collect(),
            ai: AiChannelSnapshot::default(),
            error_code: None,
            error: None,
        }
    }
}

struct MonitorRuntime {
    snapshot: HardwareMonitorSnapshot,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    session_active: bool,
}

pub struct HardwareMonitorState {
    runtime: Mutex<MonitorRuntime>,
    released: Condvar,
}

impl Default for HardwareMonitorState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(MonitorRuntime {
                snapshot: HardwareMonitorSnapshot::default(),
                cancel: Arc::new(AtomicBool::new(false)),
                worker: None,
                session_active: false,
            }),
            released: Condvar::new(),
        }
    }
}

impl HardwareMonitorState {
    fn snapshot(&self) -> HardwareMonitorSnapshot {
        self.runtime
            .lock()
            .map(|runtime| runtime.snapshot.clone())
            .unwrap_or_default()
    }

    fn publish(&self, app: &AppHandle, update: impl FnOnce(&mut HardwareMonitorSnapshot)) {
        let snapshot = match self.runtime.lock() {
            Ok(mut runtime) => {
                update(&mut runtime.snapshot);
                runtime.snapshot.updated_at_ms = now_ms();
                runtime.snapshot.clone()
            }
            Err(_) => return,
        };
        let _ = app.emit("hardware-monitor-update", snapshot);
    }

    pub fn shutdown(&self, app: &AppHandle) {
        let worker = {
            let mut runtime = match self.runtime.lock() {
                Ok(runtime) => runtime,
                Err(_) => return,
            };
            runtime.cancel.store(true, Ordering::Release);
            runtime.snapshot.state = "stopping".to_owned();
            runtime.worker.take()
        };
        if let Some(worker) = worker {
            if worker.join().is_err() {
                log::error!("hardware monitor worker panicked during shutdown");
            }
        }
        self.publish(app, |snapshot| {
            snapshot.state = "stopped".to_owned();
        });
    }

    pub fn wait_until_released(&self, timeout: Duration) -> Result<(), String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "HARDWARE_MONITOR_STATE_LOCK".to_owned())?;
        let (runtime, wait) = self
            .released
            .wait_timeout_while(runtime, timeout, |runtime| runtime.session_active)
            .map_err(|_| "HARDWARE_MONITOR_STATE_LOCK".to_owned())?;
        if wait.timed_out() && runtime.session_active {
            Err("HARDWARE_MONITOR_RELEASE_TIMEOUT".to_owned())
        } else {
            Ok(())
        }
    }

    fn set_session_active(&self, active: bool) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.session_active = active;
        }
        if !active {
            self.released.notify_all();
        }
    }
}

#[tauri::command]
pub fn hardware_monitor_status(
    state: State<'_, Arc<HardwareMonitorState>>,
) -> HardwareMonitorSnapshot {
    state.snapshot()
}

#[tauri::command]
pub fn hardware_monitor_start(
    app: AppHandle,
    state: State<'_, Arc<HardwareMonitorState>>,
    ch347: State<'_, Arc<ch347::Ch347State>>,
    supervisor: State<'_, Arc<supervisor::SupervisorState>>,
    diagnostics: State<'_, Arc<diagnostics::DiagnosticState>>,
    request: HardwareMonitorStartRequest,
) -> Result<HardwareMonitorSnapshot, String> {
    start_monitor(
        app,
        state.inner().clone(),
        ch347.inner().clone(),
        supervisor.inner().clone(),
        diagnostics.inner().clone(),
        request,
    )
}

fn start_monitor(
    app: AppHandle,
    state: Arc<HardwareMonitorState>,
    ch347: Arc<ch347::Ch347State>,
    supervisor: Arc<supervisor::SupervisorState>,
    diagnostics: Arc<diagnostics::DiagnosticState>,
    request: HardwareMonitorStartRequest,
) -> Result<HardwareMonitorSnapshot, String> {
    crate::flash_owner::check_ch347()?;
    let stale_worker = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "HARDWARE_MONITOR_STATE_LOCK".to_owned())?;
        if runtime
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Ok(runtime.snapshot.clone());
        }
        runtime.worker.take()
    };
    if let Some(worker) = stale_worker {
        let _ = worker.join();
    }

    let cancel = Arc::new(AtomicBool::new(false));
    state.publish(&app, |snapshot| {
        *snapshot = HardwareMonitorSnapshot {
            state: "starting".to_owned(),
            device_index: Some(request.device_index),
            serial_number: request.serial_number.clone(),
            relays: "board-controlled".to_owned(),
            ..HardwareMonitorSnapshot::default()
        };
    });

    let worker_state = state.clone();
    let worker_ch347 = ch347.clone();
    let worker_supervisor = supervisor.clone();
    let worker_app = app.clone();
    let worker_cancel = cancel.clone();
    let worker_request = request.clone();
    let worker_diagnostics = diagnostics.clone();
    let worker = thread::Builder::new()
        .name("lynx-hardware-monitor".to_owned())
        .spawn(move || {
            run_worker(
                worker_app,
                worker_state,
                worker_ch347,
                worker_supervisor,
                worker_cancel,
                worker_request,
                worker_diagnostics,
            );
        })
        .map_err(|error| format!("HARDWARE_MONITOR_THREAD_START:{error}"))?;
    {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "HARDWARE_MONITOR_STATE_LOCK".to_owned())?;
        runtime.cancel = cancel;
        runtime.worker = Some(worker);
    }
    Ok(state.snapshot())
}

#[tauri::command]
pub fn hardware_monitor_stop(
    app: AppHandle,
    state: State<'_, Arc<HardwareMonitorState>>,
) -> HardwareMonitorSnapshot {
    state.shutdown(&app);
    state.snapshot()
}

fn run_worker(
    app: AppHandle,
    state: Arc<HardwareMonitorState>,
    ch347: Arc<ch347::Ch347State>,
    supervisor: Arc<supervisor::SupervisorState>,
    cancel: Arc<AtomicBool>,
    request: HardwareMonitorStartRequest,
    diagnostics: Arc<diagnostics::DiagnosticState>,
) {
    let mut failures = 0u32;
    let mut last_reported_error_code = None;
    while !cancel.load(Ordering::Acquire) {
        if hardware_safety::activity_may_be_active_nonblocking(&app) {
            state.publish(&app, |snapshot| {
                snapshot.state = "paused".to_owned();
                snapshot.error_code = None;
                snapshot.error = Some("前台硬件操作优先，监视已暂停".to_owned());
            });
            wait_cancelled(&cancel, Duration::from_millis(100));
            continue;
        }
        let resolved = ch347::commands::resolve_device_index_core(
            &ch347,
            request.device_index,
            &request.serial_number,
        );
        match resolved.and_then(|index| {
            run_connected_session(
                &app,
                &state,
                &ch347,
                &supervisor,
                &cancel,
                index,
                &mut last_reported_error_code,
            )
        }) {
            Ok(()) => break,
            Err(error)
                if error == "HARDWARE_MONITOR_PREEMPTED" && !cancel.load(Ordering::Acquire) =>
            {
                state.publish(&app, |snapshot| {
                    snapshot.state = "paused".to_owned();
                    snapshot.error_code = None;
                    snapshot.error = Some("前台硬件操作优先，监视会话已释放".to_owned());
                });
                while hardware_safety::activity_may_be_active_nonblocking(&app)
                    && !cancel.load(Ordering::Acquire)
                {
                    wait_cancelled(&cancel, Duration::from_millis(50));
                }
            }
            Err(error)
                if matches!(
                    error_code(&error),
                    "CH347_OWNER_UNKNOWN" | "FLASH_OWNER_UNKNOWN"
                ) =>
            {
                state.publish(&app, |snapshot| {
                    snapshot.error_code = Some(error_code(&error).to_owned());
                    snapshot.error = Some(safe_error(&error));
                });
                break;
            }
            Err(error) if !cancel.load(Ordering::Acquire) => {
                failures = failures.saturating_add(1);
                let delay = reconnect_delay(failures);
                let code = error_code(&error);
                if is_monitor_waiting_error(code) {
                    log::debug!("hardware monitor is waiting: {error}; retry in {delay:?}");
                } else {
                    log::warn!("hardware monitor session lost: {error}; retry in {delay:?}");
                }
                if should_capture_monitor_error(code, &mut last_reported_error_code) {
                    diagnostics::capture_backend_error(
                        &diagnostics,
                        "hardware_monitor",
                        code,
                        "background_hardware_monitor",
                    );
                }
                state.publish(&app, |snapshot| {
                    snapshot.state = if is_monitor_waiting_error(code) {
                        "waiting".to_owned()
                    } else {
                        "reconnecting".to_owned()
                    };
                    snapshot.relays = "board-controlled".to_owned();
                    snapshot.reconnect_count = failures;
                    snapshot.error_code = Some(code.to_owned());
                    snapshot.error = Some(safe_error(&error));
                });
                wait_cancelled(&cancel, delay);
            }
            Err(_) => break,
        }
    }
    state.publish(&app, |snapshot| {
        snapshot.state = "stopped".to_owned();
        snapshot.relays = "board-controlled".to_owned();
    });
}

fn run_connected_session(
    app: &AppHandle,
    state: &Arc<HardwareMonitorState>,
    ch347: &Arc<ch347::Ch347State>,
    supervisor: &Arc<supervisor::SupervisorState>,
    cancel: &Arc<AtomicBool>,
    index: u32,
    last_reported_error_code: &mut Option<String>,
) -> Result<(), String> {
    ch347::commands::open_core(ch347, index)?;
    state.set_session_active(true);
    let _active_session = ActiveMonitorSession(state.clone());
    let _session = DeviceSession::new(ch347.clone(), index);
    let mut display = St7789::new(ch347, index);
    display.initialize()?;

    state.publish(app, |snapshot| {
        snapshot.state = "running".to_owned();
        snapshot.device_index = Some(index);
        snapshot.relays = "board-controlled".to_owned();
        snapshot.error_code = None;
        snapshot.error = None;
    });

    let mut identified = [false; 4];
    let mut first_frame = true;
    while !cancel.load(Ordering::Acquire) {
        if hardware_safety::activity_may_be_active_nonblocking(app) {
            return Err("HARDWARE_MONITOR_PREEMPTED".to_owned());
        }
        crate::flash_owner::check_ch347()?;
        let started = Instant::now();
        let channels: Vec<_> = (0..4)
            .map(|channel| read_channel(ch347, index, channel, &mut identified[channel]))
            .collect();
        let ai = read_ai(supervisor);
        let frame = render_dashboard(&channels, &ai);
        if first_frame {
            display.show_region(&frame, 0, 0, WIDTH, HEIGHT)?;
            first_frame = false;
        } else {
            display.show_region(&frame, 0, 0, WIDTH, HEADER_HEIGHT)?;
            for channel in 0..4 {
                let row = channel / 2;
                let column = channel % 2;
                display.show_region(
                    &frame,
                    column * 120 + 7,
                    HEADER_HEIGHT + row * 102 + 43,
                    106,
                    55,
                )?;
            }
        }
        state.publish(app, |snapshot| {
            snapshot.state = "running".to_owned();
            snapshot.sequence = snapshot.sequence.saturating_add(1);
            snapshot.channels = channels;
            snapshot.ai = ai;
            snapshot.relays = "board-controlled".to_owned();
            snapshot.error_code = None;
            snapshot.error = None;
        });
        last_reported_error_code.take();
        wait_cancelled(
            cancel,
            Duration::from_secs(1).saturating_sub(started.elapsed()),
        );
    }
    Ok(())
}

struct DeviceSession {
    state: Arc<ch347::Ch347State>,
    index: u32,
}

struct ActiveMonitorSession(Arc<HardwareMonitorState>);

impl Drop for ActiveMonitorSession {
    fn drop(&mut self) {
        self.0.set_session_active(false);
    }
}

impl DeviceSession {
    fn new(state: Arc<ch347::Ch347State>, index: u32) -> Self {
        Self { state, index }
    }
}

impl Drop for DeviceSession {
    fn drop(&mut self) {
        let bl_mask = 1u8 << 3;
        if crate::flash_owner::check_ch347().is_ok() {
            let _ = ch347::commands::gpio_set_core(&self.state, self.index, bl_mask, bl_mask, 0);
        }
        let _ = ch347::commands::close_core(&self.state, self.index);
    }
}

fn read_word(
    state: &ch347::Ch347State,
    index: u32,
    address: u8,
    register: u8,
) -> Result<[u8; 2], String> {
    let value = ch347::commands::i2c_transfer_core(
        state,
        index,
        vec![address << 1, register],
        2,
        Some(100),
        None,
        None,
    )?;
    value
        .try_into()
        .map_err(|value: Vec<u8>| format!("INA226_SHORT_READ:{address:02X}:{}", value.len()))
}

fn read_channel(
    state: &ch347::Ch347State,
    index: u32,
    channel: usize,
    identified: &mut bool,
) -> ChannelSnapshot {
    let address = I2C_ADDRESSES[channel];
    let result = (|| {
        if !*identified {
            let manufacturer = u16::from_be_bytes(read_word(state, index, address, 0xfe)?);
            let die = u16::from_be_bytes(read_word(state, index, address, 0xff)?);
            validate_identity(address, manufacturer, die)?;
            *identified = true;
        }
        let configuration = u16::from_be_bytes(read_word(state, index, address, 0x00)?);
        validate_configuration(address, configuration)?;
        let raw_bus = u16::from_be_bytes(read_word(state, index, address, 0x02)?);
        let raw_shunt = i16::from_be_bytes(read_word(state, index, address, 0x01)?);
        measurement(channel, raw_bus, raw_shunt)
    })();
    match result {
        Ok(value) => value,
        Err(error) => ChannelSnapshot::failed(channel, error_code(&error), safe_error(&error)),
    }
}

fn validate_identity(address: u8, manufacturer: u16, die: u16) -> Result<(), String> {
    if manufacturer == 0x5449 && die & 0xfff0 == 0x2260 {
        Ok(())
    } else {
        Err(format!(
            "INA226_IDENTITY_MISMATCH:{address:02X}:{manufacturer:04X}:{die:04X}"
        ))
    }
}

fn validate_configuration(address: u8, configuration: u16) -> Result<(), String> {
    if configuration & 0x07 == 0x07 {
        Ok(())
    } else {
        Err(format!(
            "INA226_CONFIGURATION_INVALID:{address:02X}:{configuration:04X}"
        ))
    }
}

fn measurement(channel: usize, raw_bus: u16, raw_shunt: i16) -> Result<ChannelSnapshot, String> {
    let address = I2C_ADDRESSES[channel];
    if raw_shunt.unsigned_abs() >= 32_700 {
        return Err(format!("INA226_SHUNT_OVER_RANGE:{address:02X}:{raw_shunt}"));
    }
    let bus_voltage_v = f64::from(raw_bus) * 1.25e-3;
    let shunt_voltage_v = f64::from(raw_shunt) * 2.5e-6;
    let current_a = shunt_voltage_v / SHUNT_OHMS;
    Ok(ChannelSnapshot {
        channel: (channel + 1) as u8,
        address,
        relay_gpio: RELAY_GPIOS[channel],
        status: "ok".to_owned(),
        bus_voltage_v: Some(bus_voltage_v),
        shunt_voltage_v: Some(shunt_voltage_v),
        current_a: Some(current_a),
        power_w: Some(bus_voltage_v * current_a),
        error_code: None,
        error: None,
    })
}

fn read_ai(state: &supervisor::SupervisorState) -> AiChannelSnapshot {
    let result = (|| {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let providers =
            supervisor::storage::list_providers(&db).map_err(|error| error.to_string())?;
        let overview = supervisor::storage::overview(&db).map_err(|error| error.to_string())?;
        let enabled = providers.iter().filter(|provider| provider.enabled).count() as u64;
        let healthy = providers
            .iter()
            .filter(|provider| {
                provider.enabled && matches!(provider.last_test_status.as_str(), "success" | "ok")
            })
            .count();
        let last_test = providers
            .iter()
            .find(|provider| provider.enabled && provider.is_default)
            .or_else(|| providers.iter().find(|provider| provider.enabled))
            .map(|provider| provider.last_test_status.clone())
            .unwrap_or_else(|| "untested".to_owned());
        let status = if overview.active_run_count > 0 {
            "active"
        } else if overview.provider_count == 0 {
            "unconfigured"
        } else if overview.ready_provider_count == 0 {
            "offline"
        } else if healthy > 0 {
            "healthy"
        } else {
            "ready"
        };
        Ok::<_, String>(AiChannelSnapshot {
            status: status.to_owned(),
            provider_count: overview.provider_count,
            enabled_provider_count: enabled,
            ready_provider_count: overview.ready_provider_count,
            active_run_count: overview.active_run_count,
            waiting_human_count: overview.waiting_human_count,
            last_test_status: last_test,
        })
    })();
    result.unwrap_or_else(|error| {
        log::warn!("hardware monitor AI status unavailable: {error}");
        AiChannelSnapshot {
            status: "error".to_owned(),
            ..AiChannelSnapshot::default()
        }
    })
}

struct St7789<'a> {
    state: &'a ch347::Ch347State,
    index: u32,
}

impl<'a> St7789<'a> {
    fn new(state: &'a ch347::Ch347State, index: u32) -> Self {
        Self { state, index }
    }

    fn gpio(&self, pin: u8, high: bool) -> Result<(), String> {
        let mask = 1u8 << pin;
        ch347::commands::gpio_set_core(
            self.state,
            self.index,
            mask,
            mask,
            if high { mask } else { 0 },
        )
    }

    fn command(&self, command: u8, data: &[u8]) -> Result<(), String> {
        self.gpio(1, false)?;
        ch347::commands::spi_write_core(self.state, self.index, vec![command], Some(0))?;
        if !data.is_empty() {
            self.gpio(1, true)?;
            ch347::commands::spi_write_core(self.state, self.index, data.to_vec(), Some(0))?;
        }
        Ok(())
    }

    fn initialize(&mut self) -> Result<(), String> {
        ch347::commands::spi_init_core(
            self.state,
            self.index,
            Some(3),
            Some(4),
            Some(0),
            Some(8),
            Some(1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )?;
        self.gpio(2, false)?;
        thread::sleep(Duration::from_millis(100));
        self.gpio(2, true)?;
        thread::sleep(Duration::from_millis(100));
        self.gpio(3, true)?;
        const INIT: &[(u8, &[u8], u64)] = &[
            (0x11, &[], 120),
            (0x36, &[0x00], 0),
            (0x3a, &[0x05], 0),
            (0x21, &[], 0),
            (0xb2, &[0x1f, 0x1f, 0x00, 0x33, 0x33], 0),
            (0xb7, &[0x12], 0),
            (0xbb, &[0x2a], 0),
            (0xc0, &[0x2c], 0),
            (0xc2, &[0x01], 0),
            (0xc3, &[0x07], 0),
            (0xc4, &[0x20], 0),
            (0xc6, &[0x13], 0),
            (0xd0, &[0xa4, 0xa1], 0),
            (0xd6, &[0xa1], 0),
            (
                0xe0,
                &[
                    0xf0, 0x06, 0x0d, 0x0b, 0x0a, 0x07, 0x2e, 0x43, 0x45, 0x38, 0x14, 0x13, 0x25,
                    0x29,
                ],
                0,
            ),
            (
                0xe1,
                &[
                    0xf0, 0x07, 0x0a, 0x08, 0x07, 0x23, 0x2e, 0x33, 0x44, 0x3a, 0x16, 0x17, 0x26,
                    0x2c,
                ],
                0,
            ),
            (0x29, &[], 100),
        ];
        for (command, data, delay_ms) in INIT {
            self.command(*command, data)?;
            if *delay_ms > 0 {
                thread::sleep(Duration::from_millis(*delay_ms));
            }
        }
        Ok(())
    }

    fn show_region(
        &self,
        frame: &[u8],
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        if x + width > WIDTH || y + height > HEIGHT || frame.len() != WIDTH * HEIGHT * 2 {
            return Err("ST7789_REGION_INVALID".to_owned());
        }
        let x_end = x + width - 1;
        let y_end = y + height - 1;
        self.command(
            0x2a,
            &[(x >> 8) as u8, x as u8, (x_end >> 8) as u8, x_end as u8],
        )?;
        self.command(
            0x2b,
            &[(y >> 8) as u8, y as u8, (y_end >> 8) as u8, y_end as u8],
        )?;
        self.command(0x2c, &[])?;
        self.gpio(1, true)?;
        let mut region = Vec::with_capacity(width * height * 2);
        for row in y..y + height {
            let start = (row * WIDTH + x) * 2;
            region.extend_from_slice(&frame[start..start + width * 2]);
        }
        ch347::commands::spi_write_chunks_core(self.state, self.index, &region, 480, Some(0))
    }
}

struct Canvas {
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(color: Rgb) -> Self {
        Self {
            pixels: rgb565(color).repeat(WIDTH * HEIGHT),
        }
    }

    fn fill(&mut self, x: usize, y: usize, width: usize, height: usize, color: Rgb) {
        let pixel = rgb565(color);
        for row in y.min(HEIGHT)..(y + height).min(HEIGHT) {
            for column in x.min(WIDTH)..(x + width).min(WIDTH) {
                let offset = (row * WIDTH + column) * 2;
                self.pixels[offset..offset + 2].copy_from_slice(&pixel);
            }
        }
    }

    fn text(&mut self, text: &str, x: usize, y: usize, color: Rgb, large: bool) {
        let (cell_width, cell_height, atlas) = if large {
            (LARGE_CELL.0, LARGE_CELL.1, &FONT_BYTES[SMALL_ATLAS_LEN..])
        } else {
            (SMALL_CELL.0, SMALL_CELL.1, &FONT_BYTES[..SMALL_ATLAS_LEN])
        };
        let mut cursor = x;
        for character in text.chars() {
            let index = DISPLAY_GLYPHS
                .find(character)
                .or_else(|| DISPLAY_GLYPHS.find('?'))
                .unwrap_or(0);
            let glyph =
                &atlas[index * cell_width * cell_height..(index + 1) * cell_width * cell_height];
            for row in 0..cell_height {
                for column in 0..cell_width {
                    let alpha = glyph[row * cell_width + column];
                    if alpha > 0 {
                        self.blend(cursor + column, y + row, color, alpha);
                    }
                }
            }
            cursor += cell_width;
        }
    }

    fn blend(&mut self, x: usize, y: usize, foreground: Rgb, alpha: u8) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let offset = (y * WIDTH + x) * 2;
        if alpha == 255 {
            self.pixels[offset..offset + 2].copy_from_slice(&rgb565(foreground));
            return;
        }
        let value = u16::from_be_bytes([self.pixels[offset], self.pixels[offset + 1]]);
        let behind = (
            (((value >> 11) & 0x1f) * 255 / 31) as u8,
            (((value >> 5) & 0x3f) * 255 / 63) as u8,
            ((value & 0x1f) * 255 / 31) as u8,
        );
        let mix = |front: u8, back: u8| -> u8 {
            ((u16::from(front) * u16::from(alpha)
                + u16::from(back) * (255 - u16::from(alpha))
                + 127)
                / 255) as u8
        };
        self.pixels[offset..offset + 2].copy_from_slice(&rgb565((
            mix(foreground.0, behind.0),
            mix(foreground.1, behind.1),
            mix(foreground.2, behind.2),
        )));
    }
}

fn render_dashboard(channels: &[ChannelSnapshot], ai: &AiChannelSnapshot) -> Vec<u8> {
    let mut canvas = Canvas::new((9, 14, 25));
    let ai_color = match ai.status.as_str() {
        "active" | "healthy" | "ready" => (42, 196, 113),
        "offline" | "error" => ERROR,
        _ => (230, 151, 0),
    };
    canvas.fill(0, 0, WIDTH, HEADER_HEIGHT, (12, 22, 38));
    canvas.fill(0, 0, 6, HEADER_HEIGHT, ai_color);
    canvas.fill(9, 7, 7, 7, ai_color);
    canvas.text(
        &format!(
            "AI {:8} {}/{} READY",
            ai.status.to_uppercase().chars().take(8).collect::<String>(),
            ai.ready_provider_count,
            ai.enabled_provider_count
        ),
        22,
        1,
        WHITE,
        false,
    );
    canvas.text(
        &format!(
            "RUN{} WAIT{} TEST:{} 1S",
            ai.active_run_count,
            ai.waiting_human_count,
            ai.last_test_status
                .to_uppercase()
                .chars()
                .take(7)
                .collect::<String>()
        ),
        9,
        18,
        MUTED,
        false,
    );
    for (index, channel) in channels.iter().enumerate() {
        let row = index / 2;
        let column = index % 2;
        let x = column * 120;
        let y = HEADER_HEIGHT + row * 102;
        canvas.fill(x, y, 120, 102, PANEL_BACKGROUNDS[index]);
        canvas.fill(x, y, 120, 4, ACCENTS[index]);
        canvas.text(&format!("CH{}", index + 1), x + 8, y + 5, WHITE, true);
        canvas.text(
            &format!("GPIO{} INA{:02X}", channel.relay_gpio, channel.address),
            x + 8,
            y + 27,
            ACCENTS[index],
            false,
        );
        if channel.status == "ok" {
            canvas.text(
                &format!("V {:7.3}V", channel.bus_voltage_v.unwrap_or_default()),
                x + 8,
                y + 44,
                WHITE,
                false,
            );
            canvas.text(
                &format!("I {:+7.3}A", channel.current_a.unwrap_or_default()),
                x + 8,
                y + 61,
                WHITE,
                false,
            );
            canvas.text(
                &format!("P {:+7.3}W", channel.power_w.unwrap_or_default()),
                x + 8,
                y + 78,
                WHITE,
                false,
            );
        } else {
            canvas.text(
                channel
                    .error_code
                    .as_deref()
                    .unwrap_or("NO DATA")
                    .chars()
                    .take(12)
                    .collect::<String>()
                    .as_str(),
                x + 8,
                y + 49,
                ERROR,
                false,
            );
            canvas.text("RETRYING", x + 8, y + 68, MUTED, false);
        }
    }
    canvas.fill(119, HEADER_HEIGHT, 2, HEIGHT - HEADER_HEIGHT, (52, 63, 72));
    canvas.fill(0, HEADER_HEIGHT + 101, WIDTH, 2, (52, 63, 72));
    canvas.pixels
}

fn rgb565(color: Rgb) -> [u8; 2] {
    let value = (u16::from(color.0 & 0xf8) << 8)
        | (u16::from(color.1 & 0xfc) << 3)
        | u16::from(color.2 >> 3);
    value.to_be_bytes()
}

fn reconnect_delay(failures: u32) -> Duration {
    Duration::from_millis(match failures {
        0 | 1 => 500,
        2 => 1_000,
        3 => 2_000,
        4 => 4_000,
        _ => 5_000,
    })
}

fn wait_cancelled(cancel: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep((deadline - Instant::now()).min(Duration::from_millis(50)));
    }
}

fn error_code(error: &str) -> &str {
    let candidate = error.split(':').next().unwrap_or_default();
    if !candidate.is_empty()
        && candidate.len() <= 64
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return candidate;
    }
    if error.contains("CH347DLL") {
        "CH347_BACKEND_UNAVAILABLE"
    } else if error.to_ascii_lowercase().contains("open device") {
        "CH347_OPEN_FAILED"
    } else if error.contains("I2C") {
        "CH347_I2C_FAILED"
    } else if error.contains("SPI") {
        "CH347_SPI_FAILED"
    } else if error.contains("GPIO") {
        "CH347_GPIO_FAILED"
    } else {
        "HARDWARE_MONITOR_FAILED"
    }
}

fn is_monitor_waiting_error(code: &str) -> bool {
    matches!(
        code,
        "CH347F_DEVICE_NOT_FOUND" | "CH347F_DEVICE_SELECTION_REQUIRED"
    )
}

fn should_capture_monitor_error(code: &str, last_reported: &mut Option<String>) -> bool {
    if is_monitor_waiting_error(code) || last_reported.as_deref() == Some(code) {
        return false;
    }
    *last_reported = Some(code.to_owned());
    true
}

fn safe_error(error: &str) -> String {
    error.chars().take(240).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_is_big_endian() {
        assert_eq!(rgb565((255, 0, 0)), [0xf8, 0x00]);
        assert_eq!(rgb565((0, 255, 0)), [0x07, 0xe0]);
    }

    #[test]
    fn dashboard_has_one_complete_frame() {
        let channels: Vec<_> = (0..4).map(ChannelSnapshot::waiting).collect();
        assert_eq!(
            render_dashboard(&channels, &AiChannelSnapshot::default()).len(),
            WIDTH * HEIGHT * 2
        );
    }

    #[test]
    fn reconnect_delay_is_bounded() {
        assert_eq!(reconnect_delay(1), Duration::from_millis(500));
        assert_eq!(reconnect_delay(4), Duration::from_secs(4));
        assert_eq!(reconnect_delay(99), Duration::from_secs(5));
    }

    #[test]
    fn monitor_diagnostics_ignore_waiting_states_and_capture_error_edges() {
        let mut last_reported = None;
        assert!(!should_capture_monitor_error(
            "CH347F_DEVICE_NOT_FOUND",
            &mut last_reported
        ));
        assert!(!should_capture_monitor_error(
            "CH347F_DEVICE_SELECTION_REQUIRED",
            &mut last_reported
        ));
        assert_eq!(last_reported, None);

        assert!(should_capture_monitor_error(
            "CH347_SPI_FAILED",
            &mut last_reported
        ));
        assert!(!should_capture_monitor_error(
            "CH347_SPI_FAILED",
            &mut last_reported
        ));
        assert!(should_capture_monitor_error(
            "CH347_GPIO_FAILED",
            &mut last_reported
        ));
        assert_eq!(last_reported.as_deref(), Some("CH347_GPIO_FAILED"));
    }

    #[test]
    fn freeform_backend_errors_map_to_stable_machine_codes() {
        assert_eq!(error_code("Failed to open device 0"), "CH347_OPEN_FAILED");
        assert_eq!(
            error_code("CH347DLL not available"),
            "CH347_BACKEND_UNAVAILABLE"
        );
        assert_eq!(
            error_code("INA226_IDENTITY_MISMATCH:40:0000:0000"),
            "INA226_IDENTITY_MISMATCH"
        );
    }

    #[test]
    fn ina226_conversion_uses_fifteen_milliohm_shunt_and_signed_voltage() {
        let positive = measurement(0, 8_000, 600).unwrap();
        assert_eq!(positive.bus_voltage_v, Some(10.0));
        assert!((positive.current_a.unwrap() - 0.1).abs() < 1e-9);
        assert!((positive.power_w.unwrap() - 1.0).abs() < 1e-9);
        let negative = measurement(3, 8_000, -600).unwrap();
        assert!((negative.current_a.unwrap() + 0.1).abs() < 1e-9);
        assert_eq!(negative.relay_gpio, 7);
    }

    #[test]
    fn generated_font_contains_antialiased_coverage() {
        assert!(FONT_BYTES.iter().any(|alpha| *alpha > 0 && *alpha < 255));
    }

    #[test]
    fn ina226_identity_and_continuous_mode_are_enforced() {
        assert!(validate_identity(0x40, 0x5449, 0x2261).is_ok());
        assert!(validate_identity(0x40, 0xffff, 0x2260).is_err());
        assert!(validate_configuration(0x40, 0x4127).is_ok());
        assert!(validate_configuration(0x40, 0x4124).is_err());
        assert!(measurement(0, 0, 32_700).is_err());
    }
}

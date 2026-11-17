//! Pluggable relay-power adapters (ASCII USB CDC and Lynx Power GPIO).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use tauri::{command, AppHandle, Manager, State};

use crate::serial::commands::{
    resolve_port_core, serial_close_core, serial_is_registered_core, SerialIdentity,
};

use super::storage;
use super::types::{AsciiCdcProfile, DeviceConfig, PowerAction};
use super::WorkbenchState;

#[derive(Clone, Copy)]
pub enum PowerAuditActor {
    User,
    Workflow,
    Supervisor,
    Gateway,
}

impl PowerAuditActor {
    fn identity(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user", "local-user"),
            Self::Workflow => ("service", "workbench-workflow"),
            Self::Supervisor => ("ai", "supervisor"),
            Self::Gateway => ("ai", "linux-mcp-gateway"),
        }
    }
}

#[command]
pub fn workbench_power_action(
    device_id: i64,
    action: PowerAction,
    pulse_ms: Option<u64>,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    let device = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::device_by_id(&db, device_id).map_err(|e| e.to_string())?
    };
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin_scoped(
        "Linux 工作台",
        "电源操作",
        format!("device:{device_id}"),
        power_resource_key(&device)?,
    )?;
    app.state::<Arc<crate::hardware_monitor::HardwareMonitorState>>()
        .wait_until_released(Duration::from_secs(2))?;
    let lock = state.device_lock(device_id)?;
    let _guard = lock
        .try_lock()
        .map_err(|_| "DEVICE_BUSY:another device operation is running".to_owned())?;
    execute_and_audit(
        &app,
        &state,
        None,
        &device,
        action,
        pulse_ms.unwrap_or(200),
        PowerAuditActor::User,
    )
}

fn power_resource_key(device: &DeviceConfig) -> Result<String, String> {
    if ch347_channel_claim(device)?.is_some() {
        // CH347's vendor driver has process-global open/GPIO state. Keep the
        // lock operation-scoped, but serialize only this adapter family.
        return Ok("ch347".to_owned());
    }
    let identity = if !device.power_controller_serial.trim().is_empty() {
        format!("serial:{}", device.power_controller_serial.trim())
    } else if !device.power_controller.trim().is_empty() {
        format!("port:{}", device.power_controller.trim())
    } else {
        format!("device:{}", device.id.unwrap_or_default())
    };
    Ok(format!("ascii-power:{identity}"))
}

pub fn execute_and_audit(
    app: &AppHandle,
    state: &WorkbenchState,
    task_id: Option<&str>,
    device: &DeviceConfig,
    action: PowerAction,
    pulse_ms: u64,
    actor: PowerAuditActor,
) -> Result<String, String> {
    let is_write = !matches!(action, PowerAction::GetState);
    if is_write {
        publish_power_intent(app, task_id, device, action, pulse_ms, actor)?;
    }
    if matches!(
        action,
        PowerAction::PowerOff | PowerAction::ResetPulse | PowerAction::BootAssert
    ) {
        release_device_sessions(app, device)?;
    }
    let result = execute(app, device, action, pulse_ms);
    if let Ok(db) = state.db.lock() {
        let (status, response) = match &result {
            Ok(response) => ("success", response.as_str()),
            Err(error) => ("failed", error.as_str()),
        };
        let _ = db.execute(
            "INSERT INTO controller_actions(task_id,device_id,action,parameters,response,status,created_at) \
             VALUES(?1,?2,?3,?4,?5,?6,unixepoch())",
            rusqlite::params![
                task_id,
                device.id.unwrap_or_default(),
                action.as_str(),
                format!("pulse_ms={pulse_ms}"),
                response,
                status
            ],
        );
    }
    if is_write {
        publish_power_result(app, task_id, device, action, actor, &result);
    }
    result
}

/// Release sessions owned by the board before an action that can remove or
/// reset its USB functions. This helper is intentionally idempotent so a
/// workflow may explicitly release its console before entering flash mode and
/// still pass through the same global safety boundary.
pub(crate) fn release_device_sessions(
    app: &AppHandle,
    device: &DeviceConfig,
) -> Result<(), String> {
    let identity = SerialIdentity {
        port: device.serial_port.clone(),
        vid: device.serial_vid,
        pid: device.serial_pid,
        serial_number: device.serial_number.clone(),
        interface_number: device.serial_interface_number,
    };
    let port = resolve_port_core(&identity)
        .map(|port| port.name)
        .unwrap_or_else(|_| device.serial_port.clone());
    if !port.trim().is_empty() {
        let serial = app.state::<Arc<crate::serial::SerialState>>();
        if serial_is_registered_core(serial.inner().as_ref(), &port) {
            serial_close_core(serial.inner().as_ref(), &port)
                .map_err(|error| format!("BOARD_SERIAL_RELEASE_FAILED:{port}:{error}"))?;
        }
    }
    Ok(())
}

fn publish_power_intent(
    app: &AppHandle,
    task_id: Option<&str>,
    device: &DeviceConfig,
    action: PowerAction,
    pulse_ms: u64,
    actor: PowerAuditActor,
) -> Result<(), String> {
    let (actor_type, actor_id) = actor.identity();
    crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "power".to_owned(),
            action: "power_action_intended".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", device.id.unwrap_or_default()),
            session_id: task_id.map(str::to_owned),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: task_id.map(str::to_owned),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: format!("准备执行受控电源动作：{}", action.as_str()),
            metadata: serde_json::json!({
                "action": action.as_str(),
                "pulseMs": pulse_ms,
                "taskBound": task_id.is_some(),
            }),
        },
    )?;
    Ok(())
}

fn publish_power_result(
    app: &AppHandle,
    task_id: Option<&str>,
    device: &DeviceConfig,
    action: PowerAction,
    actor: PowerAuditActor,
    result: &Result<String, String>,
) {
    let (actor_type, actor_id) = actor.identity();
    let (status, error_code, summary) = match result {
        Ok(_) => (
            "success",
            None,
            format!("电源动作完成：{}", action.as_str()),
        ),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "POWER_ACTION_FAILED",
            )),
            format!("电源动作失败：{}", action.as_str()),
        ),
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "power".to_owned(),
            action: "power_action_completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", device.id.unwrap_or_default()),
            session_id: task_id.map(str::to_owned),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: task_id.map(str::to_owned),
            causation_id: None,
            result: status.to_owned(),
            error_code,
            summary,
            metadata: serde_json::json!({"action": action.as_str()}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

pub fn last_known_power_state(state: &WorkbenchState, device_id: i64) -> Result<String, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let action = db
        .query_row(
            "SELECT action FROM controller_actions WHERE device_id=?1 AND status='success' \
             AND action IN ('power_on','power_off') ORDER BY created_at DESC,id DESC LIMIT 1",
            [device_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(match action.as_deref() {
        Some("power_on") => "on",
        Some("power_off") => "off",
        _ => "unknown",
    }
    .to_owned())
}

/// Return the last persisted successful power command for a saved Linux board.
/// This is intentionally a database-only projection: write-only relay adapters
/// cannot provide a physical readback and selecting a board must not touch
/// hardware or contend with an active task.
#[command]
pub fn workbench_power_last_known_state(
    device_id: i64,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        storage::device_by_id(&db, device_id).map_err(|_| "POWER_DEVICE_NOT_FOUND".to_owned())?;
    }
    last_known_power_state(state.inner().as_ref(), device_id)
}

pub fn execute(
    app: &AppHandle,
    device: &DeviceConfig,
    action: PowerAction,
    pulse_ms: u64,
) -> Result<String, String> {
    if device.power_profile_json.trim().is_empty() {
        return Err(
            "POWER_PROFILE_MISSING:configure an ASCII CDC or Lynx Power profile".to_owned(),
        );
    }
    let value: serde_json::Value = serde_json::from_str(&device.power_profile_json)
        .map_err(|e| format!("POWER_PROFILE_INVALID:{e}"))?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some("ch347_gpio") {
        let profile: Ch347GpioProfile =
            serde_json::from_value(value).map_err(|e| format!("POWER_PROFILE_INVALID:{e}"))?;
        return execute_ch347(app, device, &profile, action, pulse_ms);
    }
    execute_ascii(device, action, pulse_ms)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ch347GpioProfile {
    #[serde(rename = "type")]
    _kind: String,
    #[serde(default)]
    device_index: u32,
    #[serde(default = "default_gpio")]
    gpio: u8,
    #[serde(default = "default_true")]
    active_high: bool,
    #[serde(default)]
    role_mode: Ch347RoleMode,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Ch347RoleMode {
    #[default]
    Independent,
    SingleRelay,
    DualPower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ch347ActionPlan {
    pin: u8,
    first_asserted: bool,
    final_asserted: Option<bool>,
    final_state: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ch347GpioShadow {
    enable: u8,
    dir_out: u8,
    data_out: u8,
}

static CH347_GPIO_SHADOWS: LazyLock<Mutex<HashMap<String, Ch347GpioShadow>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn default_true() -> bool {
    true
}

fn default_gpio() -> u8 {
    4
}

pub(crate) fn validate_configuration_profile(device: &DeviceConfig) -> Result<(), String> {
    if device.power_profile_json.trim().is_empty() {
        return Ok(());
    }
    let profile_type = serde_json::from_str::<serde_json::Value>(&device.power_profile_json)
        .map_err(|error| format!("POWER_PROFILE_INVALID:{error}"))?
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("ascii_cdc")
        .to_owned();
    if profile_type != "ch347_gpio" {
        return Ok(());
    }
    let profile: Ch347GpioProfile = serde_json::from_str(&device.power_profile_json)
        .map_err(|error| format!("POWER_PROFILE_INVALID:{error}"))?;
    let power = parse_gpio(&device.power_channel, profile.gpio)?;
    match profile.role_mode {
        Ch347RoleMode::Independent => {
            let mut channels = vec![("power", power)];
            for (purpose, value) in [
                ("boot", device.boot_channel.as_str()),
                ("reset", device.reset_channel.as_str()),
            ] {
                if !value.trim().is_empty() {
                    let pin = parse_required_gpio(value, purpose)?;
                    if let Some((other, _)) = channels.iter().find(|(_, current)| *current == pin) {
                        return Err(format!(
                            "POWER_CHANNEL_CONFLICT:{other} and {purpose} GPIO must be different"
                        ));
                    }
                    channels.push((purpose, pin));
                }
            }
        }
        Ch347RoleMode::DualPower => {
            let secondary = parse_required_gpio(&device.boot_channel, "secondary power")?;
            if power == secondary {
                return Err(
                    "POWER_CHANNEL_CONFLICT:primary and secondary power GPIO must be different"
                        .to_owned(),
                );
            }
        }
        Ch347RoleMode::SingleRelay => {}
    }
    Ok(())
}

pub(crate) fn ch347_channel_claim(
    device: &DeviceConfig,
) -> Result<Option<(String, Vec<u8>)>, String> {
    if device.power_profile_json.trim().is_empty() {
        return Ok(None);
    }
    let profile: Ch347GpioProfile =
        match serde_json::from_str::<Ch347GpioProfile>(&device.power_profile_json) {
            Ok(profile) if profile._kind == "ch347_gpio" => profile,
            Ok(_) => return Ok(None),
            Err(error) => return Err(format!("POWER_PROFILE_INVALID:{error}")),
        };
    let controller = if device.power_controller_serial.trim().is_empty() {
        format!("index:{}", profile.device_index)
    } else {
        format!("serial:{}", device.power_controller_serial.trim())
    };
    let power = parse_gpio(&device.power_channel, profile.gpio)?;
    let mut channels = vec![power];
    match profile.role_mode {
        Ch347RoleMode::Independent => {
            for value in [&device.boot_channel, &device.reset_channel] {
                if !value.trim().is_empty() {
                    channels.push(parse_required_gpio(value, "board control")?);
                }
            }
        }
        Ch347RoleMode::DualPower => channels.push(parse_required_gpio(
            &device.boot_channel,
            "secondary power",
        )?),
        Ch347RoleMode::SingleRelay => {}
    }
    channels.sort_unstable();
    channels.dedup();
    Ok(Some((controller, channels)))
}

pub(crate) fn validate_workflow_profile(device: &DeviceConfig) -> Result<(), String> {
    validate_configuration_profile(device)?;
    let profile_type = serde_json::from_str::<serde_json::Value>(&device.power_profile_json)
        .map_err(|error| format!("POWER_PROFILE_INVALID:{error}"))?
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("ascii_cdc")
        .to_owned();
    if profile_type == "ch347_gpio" {
        let profile: Ch347GpioProfile = serde_json::from_str(&device.power_profile_json)
            .map_err(|error| format!("POWER_PROFILE_INVALID:{error}"))?;
        if profile.role_mode == Ch347RoleMode::Independent {
            parse_required_gpio(&device.boot_channel, "boot")?;
        }
    } else {
        let profile: AsciiCdcProfile = serde_json::from_str(&device.power_profile_json)
            .map_err(|error| format!("POWER_PROFILE_INVALID:{error}"))?;
        for (action, command) in [
            ("get_state", profile.get_state.as_str()),
            ("power_off", profile.power_off.as_str()),
            ("boot_assert", profile.boot_assert.as_str()),
            ("power_on", profile.power_on.as_str()),
            ("boot_release", profile.boot_release.as_str()),
        ] {
            if command.trim().is_empty() {
                return Err(format!("POWER_ACTION_UNSUPPORTED:{action}"));
            }
        }
    }
    Ok(())
}

fn ch347_action_plan(
    device: &DeviceConfig,
    profile: &Ch347GpioProfile,
    action: PowerAction,
) -> Result<Ch347ActionPlan, String> {
    let coordinated_power = profile.role_mode != Ch347RoleMode::Independent;
    let pin = if coordinated_power {
        parse_gpio(&device.power_channel, profile.gpio)?
    } else {
        match action {
            PowerAction::BootAssert | PowerAction::BootRelease => {
                parse_required_gpio(&device.boot_channel, "boot")?
            }
            PowerAction::ResetPulse => parse_required_gpio(&device.reset_channel, "reset")?,
            _ => parse_gpio(&device.power_channel, profile.gpio)?,
        }
    };
    let plan = match (coordinated_power, action) {
        (_, PowerAction::GetState) => Ch347ActionPlan {
            pin,
            first_asserted: false,
            final_asserted: None,
            final_state: "unknown",
        },
        (true, PowerAction::PowerOn | PowerAction::BootRelease) => Ch347ActionPlan {
            pin,
            first_asserted: true,
            final_asserted: None,
            final_state: "on",
        },
        (true, PowerAction::PowerOff | PowerAction::BootAssert) => Ch347ActionPlan {
            pin,
            first_asserted: false,
            final_asserted: None,
            final_state: "off",
        },
        (true, PowerAction::ResetPulse) => Ch347ActionPlan {
            pin,
            first_asserted: false,
            final_asserted: Some(true),
            final_state: "on",
        },
        (false, PowerAction::PowerOn | PowerAction::BootAssert) => Ch347ActionPlan {
            pin,
            first_asserted: true,
            final_asserted: None,
            final_state: "on",
        },
        (false, PowerAction::PowerOff | PowerAction::BootRelease) => Ch347ActionPlan {
            pin,
            first_asserted: false,
            final_asserted: None,
            final_state: "off",
        },
        (false, PowerAction::ResetPulse) => Ch347ActionPlan {
            pin,
            first_asserted: true,
            final_asserted: Some(false),
            final_state: "off",
        },
    };
    Ok(plan)
}

fn ch347_action_mask(
    device: &DeviceConfig,
    profile: &Ch347GpioProfile,
    plan: &Ch347ActionPlan,
) -> Result<u8, String> {
    let mut mask = 1u8 << plan.pin;
    if profile.role_mode == Ch347RoleMode::DualPower {
        let secondary = parse_required_gpio(&device.boot_channel, "secondary power")?;
        if secondary == plan.pin {
            return Err(
                "POWER_CHANNEL_CONFLICT:primary and secondary power GPIO must be different"
                    .to_owned(),
            );
        }
        mask |= 1u8 << secondary;
    }
    Ok(mask)
}

fn execute_ch347(
    app: &AppHandle,
    device: &DeviceConfig,
    profile: &Ch347GpioProfile,
    action: PowerAction,
    pulse_ms: u64,
) -> Result<String, String> {
    let plan = ch347_action_plan(device, profile, action)?;
    let pin = plan.pin;
    let mask = ch347_action_mask(device, profile, &plan)?;
    let channels = if profile.role_mode == Ch347RoleMode::DualPower {
        format!(
            "{pin},{}",
            parse_required_gpio(&device.boot_channel, "secondary power")?
        )
    } else {
        pin.to_string()
    };
    if matches!(action, PowerAction::GetState) {
        return Ok(format!(
            "state=unknown;source=write_only;adapter=lynx_power;channels={channels}"
        ));
    }
    let ch347 = app.state::<Arc<crate::ch347::Ch347State>>();
    let controller_index = crate::ch347::commands::resolve_device_index_core(
        ch347.inner().as_ref(),
        profile.device_index,
        &device.power_controller_serial,
    )
    .map_err(|error| format!("POWER_ADAPTER_RESOLVE:{error}"))?;
    crate::ch347::commands::open_core(ch347.inner().as_ref(), controller_index)
        .map_err(|error| format!("POWER_ADAPTER_OPEN:{error}"))?;
    let controller_key = if device.power_controller_serial.trim().is_empty() {
        format!("index:{controller_index}")
    } else {
        format!("serial:{}", device.power_controller_serial.trim())
    };
    let result = (|| {
        set_ch347_gpio_mask(
            ch347.inner().as_ref(),
            profile,
            controller_index,
            &controller_key,
            mask,
            plan.first_asserted,
        )?;
        if let Some(final_asserted) = plan.final_asserted {
            std::thread::sleep(Duration::from_millis(pulse_ms.clamp(1, 30_000)));
            set_ch347_gpio_mask(
                ch347.inner().as_ref(),
                profile,
                controller_index,
                &controller_key,
                mask,
                final_asserted,
            )?;
        }
        Ok(format!(
            "state={};source=commanded;adapter=lynx_power;controllerIndex={};channels={channels};activeHigh={}",
            plan.final_state,
            controller_index,
            profile.active_high
        ))
    })();
    let close_result = crate::ch347::commands::close_core(ch347.inner().as_ref(), controller_index)
        .map_err(|error| format!("POWER_ADAPTER_CLOSE:{error}"));
    match (result, close_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(response), Ok(())) => Ok(response),
    }
}

fn parse_gpio(value: &str, fallback: u8) -> Result<u8, String> {
    let pin = if value.trim().is_empty() {
        fallback
    } else {
        value
            .trim()
            .trim_start_matches("GPIO")
            .parse::<u8>()
            .map_err(|_| format!("POWER_CHANNEL_INVALID:{value}"))?
    };
    if !(4..=7).contains(&pin) {
        return Err(format!("POWER_CHANNEL_INVALID:{pin}:expected 4..7"));
    }
    Ok(pin)
}

fn parse_required_gpio(value: &str, purpose: &str) -> Result<u8, String> {
    if value.trim().is_empty() {
        return Err(format!(
            "POWER_CHANNEL_MISSING:{purpose} channel must be configured"
        ));
    }
    parse_gpio(value, 0)
}

fn set_ch347_gpio_mask(
    state: &crate::ch347::Ch347State,
    profile: &Ch347GpioProfile,
    controller_index: u32,
    controller_key: &str,
    mask: u8,
    asserted: bool,
) -> Result<(), String> {
    let high = if profile.active_high {
        asserted
    } else {
        !asserted
    };
    let mut shadows = CH347_GPIO_SHADOWS
        .lock()
        .map_err(|_| "POWER_ADAPTER_STATE_LOCK".to_owned())?;
    let current = shadows.get(controller_key).copied().unwrap_or_default();
    let next = update_gpio_shadow(current, mask, high);
    crate::ch347::commands::gpio_set_core(
        state,
        controller_index,
        next.enable,
        next.dir_out,
        next.data_out,
    )
    .map_err(|error| format!("POWER_ADAPTER_CHANNEL:{error}"))?;
    shadows.insert(controller_key.to_owned(), next);
    Ok(())
}

fn update_gpio_shadow(current: Ch347GpioShadow, mask: u8, high: bool) -> Ch347GpioShadow {
    Ch347GpioShadow {
        enable: current.enable | mask,
        dir_out: current.dir_out | mask,
        data_out: if high {
            current.data_out | mask
        } else {
            current.data_out & !mask
        },
    }
}

fn execute_ascii(
    device: &DeviceConfig,
    action: PowerAction,
    pulse_ms: u64,
) -> Result<String, String> {
    let profile: AsciiCdcProfile = serde_json::from_str(&device.power_profile_json)
        .map_err(|e| format!("POWER_PROFILE_INVALID:{e}"))?;
    let command = command_for(&profile, action);
    if command.trim().is_empty() {
        return Err(format!("POWER_ACTION_UNSUPPORTED:{}", action.as_str()));
    }
    let identity = SerialIdentity {
        port: device.power_controller.clone(),
        vid: device.power_controller_vid,
        pid: device.power_controller_pid,
        serial_number: device.power_controller_serial.clone(),
        interface_number: device.power_controller_interface_number,
    };
    let resolved = resolve_port_core(&identity).map_err(|e| format!("POWER_{e}"))?;
    let timeout = Duration::from_millis(profile.timeout_ms.clamp(100, 30_000));
    let mut port = serialport::new(&resolved.name, device.power_baud_rate.max(1200))
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("POWER_OPEN:{}:{e}", resolved.name))?;
    let rendered = render(command, device, pulse_ms);
    let ending = decode_line_ending(&profile.line_ending)?;
    port.write_all(rendered.as_bytes())
        .and_then(|_| port.write_all(ending.as_bytes()))
        .and_then(|_| port.flush())
        .map_err(|e| format!("POWER_WRITE:{e}"))?;

    let deadline = Instant::now() + timeout;
    let mut response = Vec::new();
    let mut buffer = [0u8; 256];
    while Instant::now() < deadline {
        match port.read(&mut buffer) {
            Ok(0) => {}
            Ok(count) => {
                response.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&response);
                if let Some(value) = validate_response(&profile, action, &text)? {
                    return Ok(value);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(format!("POWER_READ:{error}")),
        }
    }
    Err(format!(
        "POWER_TIMEOUT:{}:response={}",
        action.as_str(),
        String::from_utf8_lossy(&response).trim()
    ))
}

fn validate_response(
    profile: &AsciiCdcProfile,
    action: PowerAction,
    response: &str,
) -> Result<Option<String>, String> {
    if !profile.failure_pattern.trim().is_empty() {
        let failure =
            Regex::new(&profile.failure_pattern).map_err(|e| format!("POWER_FAILURE_REGEX:{e}"))?;
        if failure.is_match(response) {
            return Err(format!("POWER_DEVICE_ERROR:{}", response.trim()));
        }
    }
    let success =
        Regex::new(&profile.success_pattern).map_err(|e| format!("POWER_SUCCESS_REGEX:{e}"))?;
    if success.is_match(response)
        || (matches!(action, PowerAction::GetState) && !response.trim().is_empty())
    {
        Ok(Some(response.trim().to_owned()))
    } else {
        Ok(None)
    }
}

fn command_for(profile: &AsciiCdcProfile, action: PowerAction) -> &str {
    match action {
        PowerAction::PowerOn => &profile.power_on,
        PowerAction::PowerOff => &profile.power_off,
        PowerAction::BootAssert => &profile.boot_assert,
        PowerAction::BootRelease => &profile.boot_release,
        PowerAction::ResetPulse => &profile.reset_pulse,
        PowerAction::GetState => &profile.get_state,
    }
}

fn render(template: &str, device: &DeviceConfig, pulse_ms: u64) -> String {
    template
        .replace("{power_channel}", &device.power_channel)
        .replace("{boot_channel}", &device.boot_channel)
        .replace("{reset_channel}", &device.reset_channel)
        .replace("{pulse_ms}", &pulse_ms.to_string())
}

fn decode_line_ending(value: &str) -> Result<String, String> {
    match value {
        "\\n" => Ok("\n".to_owned()),
        "\\r" => Ok("\r".to_owned()),
        "\\r\\n" => Ok("\r\n".to_owned()),
        "" => Ok(String::new()),
        other if !other.contains('\\') => Ok(other.to_owned()),
        _ => Err("POWER_PROFILE_INVALID:lineEnding must be \\\\n, \\\\r or \\\\r\\\\n".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_migrated_database() -> WorkbenchState {
        let db = rusqlite::Connection::open_in_memory().expect("open test database");
        storage::migrate(&db).expect("migrate test database");
        WorkbenchState {
            db: Mutex::new(db),
            cancellations: Mutex::new(HashMap::new()),
            device_locks: Mutex::new(HashMap::new()),
            flasher_locks: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            ssh_terminals: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn power_audit_actor_identity_is_explicit() {
        assert_eq!(PowerAuditActor::User.identity(), ("user", "local-user"));
        assert_eq!(
            PowerAuditActor::Workflow.identity(),
            ("service", "workbench-workflow")
        );
        assert_eq!(PowerAuditActor::Supervisor.identity(), ("ai", "supervisor"));
        assert_eq!(
            PowerAuditActor::Gateway.identity(),
            ("ai", "linux-mcp-gateway")
        );
    }

    #[test]
    fn last_known_power_state_uses_only_latest_successful_on_or_off_action() {
        let state = state_with_migrated_database();
        assert_eq!(last_known_power_state(&state, 7).unwrap(), "unknown");
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT INTO controller_actions(device_id,action,parameters,response,status,created_at) VALUES(7,'power_on','','failed','failed',1)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO controller_actions(device_id,action,parameters,response,status,created_at) VALUES(7,'power_off','','state=off','success',2)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO controller_actions(device_id,action,parameters,response,status,created_at) VALUES(7,'get_state','','state=unknown','success',3)",
                [],
            )
            .unwrap();
        }
        assert_eq!(last_known_power_state(&state, 7).unwrap(), "off");
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT INTO controller_actions(device_id,action,parameters,response,status,created_at) VALUES(7,'power_on','','state=on','success',4)",
                [],
            )
            .unwrap();
        }
        assert_eq!(last_known_power_state(&state, 7).unwrap(), "on");
    }

    #[test]
    fn renders_controller_variables() {
        let mut device: DeviceConfig = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "board", "serialPort": "", "serialVid": 0,
            "serialPid": 0, "serialNumber": "", "baudRate": 115200,
            "flasherKind": "openixcli", "flasherPath": "", "flasherArgs": "",
            "powerController": "", "powerControllerVid": 0, "powerControllerPid": 0,
            "powerControllerSerial": "", "powerBaudRate": 115200,
            "powerProfileJson": "", "powerChannel": "P1", "bootChannel": "B2",
            "resetChannel": "R3", "powerOffWaitMs": 1000, "powerOnWaitMs": 2000,
            "serialReconnectTimeoutMs": 30000, "bootTimeoutMs": 60000
        }))
        .unwrap();
        device.power_channel = "7".to_owned();
        assert_eq!(
            render("POWER {power_channel} {pulse_ms}", &device, 250),
            "POWER 7 250"
        );
    }

    fn profile() -> AsciiCdcProfile {
        serde_json::from_value(serde_json::json!({
            "successPattern": "(?i)ok",
            "failurePattern": "(?i)(error|fail)"
        }))
        .unwrap()
    }

    #[test]
    fn controller_failure_response_is_authoritative() {
        let error =
            validate_response(&profile(), PowerAction::PowerOn, "ERROR channel").unwrap_err();
        assert!(error.starts_with("POWER_DEVICE_ERROR:"));
    }

    #[test]
    fn state_query_accepts_non_empty_hardware_state() {
        assert_eq!(
            validate_response(&profile(), PowerAction::GetState, "POWER=OFF BOOT=0").unwrap(),
            Some("POWER=OFF BOOT=0".to_owned())
        );
    }

    #[test]
    fn incomplete_response_keeps_waiting() {
        assert_eq!(
            validate_response(&profile(), PowerAction::PowerOff, "pending").unwrap(),
            None
        );
    }

    #[test]
    fn ch347_gpio_profile_defaults_and_channels_are_validated() {
        let defaults: Ch347GpioProfile = serde_json::from_value(serde_json::json!({
            "type": "ch347_gpio"
        }))
        .unwrap();
        assert_eq!(defaults.gpio, 4);
        assert!(defaults.active_high);
        assert_eq!(defaults.role_mode, Ch347RoleMode::Independent);

        let profile: Ch347GpioProfile = serde_json::from_value(serde_json::json!({
            "type": "ch347_gpio",
            "deviceIndex": 2,
            "gpio": 5,
            "activeHigh": false
        }))
        .unwrap();
        assert_eq!(profile.device_index, 2);
        assert_eq!(profile.gpio, 5);
        assert!(!profile.active_high);
        assert_eq!(parse_gpio("GPIO7", profile.gpio).unwrap(), 7);
        assert_eq!(parse_gpio("", profile.gpio).unwrap(), 5);
        assert!(parse_gpio("3", profile.gpio)
            .unwrap_err()
            .contains("expected 4..7"));
    }

    #[test]
    fn boot_and_reset_gpio_channels_must_be_explicit() {
        assert_eq!(
            parse_required_gpio("", "boot").unwrap_err(),
            "POWER_CHANNEL_MISSING:boot channel must be configured"
        );
        assert_eq!(parse_required_gpio("GPIO6", "reset").unwrap(), 6);
    }

    fn ch347_device(profile: serde_json::Value) -> DeviceConfig {
        let mut device: DeviceConfig = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "board", "serialPort": "", "serialVid": 0,
            "serialPid": 0, "serialNumber": "", "baudRate": 115200,
            "flasherKind": "openixcli", "flasherPath": "", "flasherArgs": "",
            "powerController": "lynx-power:0", "powerControllerVid": 0,
            "powerControllerPid": 0, "powerControllerSerial": "",
            "powerBaudRate": 115200, "powerProfileJson": profile.to_string(),
            "powerChannel": "6", "bootChannel": "6", "resetChannel": "6",
            "powerOffWaitMs": 1000, "powerOnWaitMs": 2000,
            "serialReconnectTimeoutMs": 30000, "bootTimeoutMs": 60000
        }))
        .unwrap();
        device.power_profile_json = profile.to_string();
        device
    }

    #[test]
    fn single_relay_mode_allows_one_gpio_for_all_roles() {
        let device = ch347_device(serde_json::json!({
            "type": "ch347_gpio",
            "deviceIndex": 0,
            "gpio": 6,
            "activeHigh": true,
            "roleMode": "singleRelay"
        }));
        assert!(validate_configuration_profile(&device).is_ok());
        assert!(validate_workflow_profile(&device).is_ok());
    }

    #[test]
    fn independent_mode_still_rejects_duplicate_role_gpio() {
        let device = ch347_device(serde_json::json!({
            "type": "ch347_gpio",
            "gpio": 6,
            "activeHigh": true,
            "roleMode": "independent"
        }));
        assert_eq!(
            validate_configuration_profile(&device).unwrap_err(),
            "POWER_CHANNEL_CONFLICT:power and boot GPIO must be different"
        );
    }

    #[test]
    fn dual_power_mode_requires_two_distinct_gpio_rails() {
        let mut device = ch347_device(serde_json::json!({
            "type": "ch347_gpio",
            "gpio": 4,
            "activeHigh": true,
            "roleMode": "dualPower"
        }));
        device.power_channel = "4".to_owned();
        device.boot_channel = "5".to_owned();
        device.reset_channel.clear();
        assert!(validate_configuration_profile(&device).is_ok());
        assert!(validate_workflow_profile(&device).is_ok());

        device.boot_channel = "4".to_owned();
        assert_eq!(
            validate_configuration_profile(&device).unwrap_err(),
            "POWER_CHANNEL_CONFLICT:primary and secondary power GPIO must be different"
        );
    }

    #[test]
    fn dual_power_actions_use_one_atomic_gpio_mask() {
        let mut device = ch347_device(serde_json::json!({
            "type": "ch347_gpio",
            "gpio": 4,
            "activeHigh": true,
            "roleMode": "dualPower"
        }));
        device.power_channel = "4".to_owned();
        device.boot_channel = "5".to_owned();
        device.reset_channel.clear();
        let profile: Ch347GpioProfile = serde_json::from_str(&device.power_profile_json).unwrap();

        for action in [
            PowerAction::PowerOn,
            PowerAction::PowerOff,
            PowerAction::BootAssert,
            PowerAction::BootRelease,
            PowerAction::ResetPulse,
        ] {
            let plan = ch347_action_plan(&device, &profile, action).unwrap();
            assert_eq!(plan.pin, 4);
            assert_eq!(
                ch347_action_mask(&device, &profile, &plan).unwrap(),
                0b0011_0000
            );
        }
    }

    #[test]
    fn ascii_controller_save_validation_remains_backward_compatible() {
        let device = ch347_device(serde_json::json!({
            "powerOn": "POWER ON"
        }));
        assert!(validate_configuration_profile(&device).is_ok());
        assert_eq!(
            validate_workflow_profile(&device).unwrap_err(),
            "POWER_ACTION_UNSUPPORTED:get_state"
        );
    }

    #[test]
    fn single_relay_reset_and_boot_actions_preserve_power_cycle_semantics() {
        let device = ch347_device(serde_json::json!({
            "type": "ch347_gpio",
            "gpio": 6,
            "activeHigh": true,
            "roleMode": "singleRelay"
        }));
        let profile: Ch347GpioProfile = serde_json::from_str(&device.power_profile_json).unwrap();
        assert_eq!(
            ch347_action_plan(&device, &profile, PowerAction::BootAssert).unwrap(),
            Ch347ActionPlan {
                pin: 6,
                first_asserted: false,
                final_asserted: None,
                final_state: "off"
            }
        );
        assert_eq!(
            ch347_action_plan(&device, &profile, PowerAction::BootRelease).unwrap(),
            Ch347ActionPlan {
                pin: 6,
                first_asserted: true,
                final_asserted: None,
                final_state: "on"
            }
        );
        assert_eq!(
            ch347_action_plan(&device, &profile, PowerAction::ResetPulse).unwrap(),
            Ch347ActionPlan {
                pin: 6,
                first_asserted: false,
                final_asserted: Some(true),
                final_state: "on"
            }
        );
    }

    #[test]
    fn ch347_gpio_updates_preserve_other_power_control_pins() {
        let power = 1 << 4;
        let boot = 1 << 5;
        let reset = 1 << 6;

        let state = update_gpio_shadow(Ch347GpioShadow::default(), power, false);
        let state = update_gpio_shadow(state, boot, true);
        assert_eq!(state.enable, power | boot);
        assert_eq!(state.dir_out, power | boot);
        assert_eq!(state.data_out, boot);

        let state = update_gpio_shadow(state, power, true);
        assert_eq!(state.data_out, power | boot);
        let state = update_gpio_shadow(state, reset, true);
        let state = update_gpio_shadow(state, reset, false);
        assert_eq!(state.enable, power | boot | reset);
        assert_eq!(state.data_out, power | boot);

        let state = update_gpio_shadow(state, boot, false);
        assert_eq!(state.data_out, power);
    }
}

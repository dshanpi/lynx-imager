//! Persisted one-click build, transfer, flash, reboot, and boot validation flow.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{command, AppHandle, Manager, State};

use crate::serial::commands::{
    resolve_port_core, serial_close_core, serial_is_open_core, serial_open_core, serial_read_core,
    SerialIdentity,
};
use crate::serial::SerialState;

use super::commands::{self, emit_status};
use super::flasher;
use super::power;
use super::ssh;
use super::storage;
use super::types::{DeviceConfig, PowerAction, ProjectConfig, SshCredentials};
use super::WorkbenchState;

#[command]
pub fn workbench_start_workflow(
    project_id: i64,
    device_id: i64,
    credentials: SshCredentials,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
    serial_state: State<'_, Arc<SerialState>>,
) -> Result<String, String> {
    start_workflow(
        project_id,
        device_id,
        credentials,
        app,
        state.inner(),
        serial_state.inner(),
    )
}

pub(crate) fn start_workflow(
    project_id: i64,
    device_id: i64,
    credentials: SshCredentials,
    app: AppHandle,
    state: &Arc<WorkbenchState>,
    serial_state: &Arc<SerialState>,
) -> Result<String, String> {
    let (host, project, device) = {
        let db = state
            .db
            .lock()
            .map_err(|_| "WORKBENCH_DATABASE_BUSY".to_owned())?;
        let project = storage::project_by_id(&db, project_id).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                format!("WORKBENCH_PROJECT_NOT_FOUND:{project_id}")
            }
            _ => "WORKBENCH_PROJECT_LOOKUP_FAILED".to_owned(),
        })?;
        if project.device_id != Some(device_id) {
            return Err("WORKBENCH_PROJECT_DEVICE_SCOPE_MISMATCH".to_owned());
        }
        let host = storage::host_by_id(&db, project.host_id).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                format!("WORKBENCH_HOST_NOT_FOUND:{}", project.host_id)
            }
            _ => "WORKBENCH_HOST_LOOKUP_FAILED".to_owned(),
        })?;
        let device = storage::device_by_id(&db, device_id).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                format!("WORKBENCH_DEVICE_NOT_FOUND:{device_id}")
            }
            _ => "WORKBENCH_DEVICE_LOOKUP_FAILED".to_owned(),
        })?;
        (host, project, device)
    };
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let hardware_guard = safety.try_begin_scoped(
        "Linux 工作台",
        "自动工作流",
        format!("project:{project_id}/device:{device_id}"),
        flasher::flasher_resource_key(&device.flasher_kind, &device.usb_binding),
    )?;
    let credentials = super::credentials::resolve(host.id, credentials)?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }
    if device.power_profile_json.trim().is_empty() {
        return Err("POWER_PROFILE_MISSING:automatic workflow requires a controller".to_owned());
    }
    power::validate_workflow_profile(&device)?;
    let task_id = commands::new_task_id("workflow");
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO task_history(task_id,kind,project_id,device_id,status,started_at,phase) \
             VALUES(?1,'workflow',?2,?3,'running',unixepoch(),'workflow')",
            rusqlite::params![task_id, project_id, device_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let workbench = state.clone();
    let serial = serial_state.clone();
    let returned = task_id.clone();
    std::thread::spawn(move || {
        let _hardware_guard = hardware_guard;
        let lock = match workbench.device_lock(device_id) {
            Ok(lock) => lock,
            Err(error) => {
                finish(&workbench, &task_id, "failed", &error);
                return;
            }
        };
        let _guard = match lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let error = "DEVICE_BUSY:another device operation is running";
                finish(&workbench, &task_id, "failed", error);
                emit_status(&app, &task_id, "preflight", "failed", error);
                return;
            }
        };
        let result = run(
            &app,
            &workbench,
            &serial,
            &task_id,
            &host,
            &project,
            &device,
            &credentials,
            &cancellation,
        );
        let (status, error) = match result {
            Ok(()) => ("success", String::new()),
            Err(error) if cancellation.load(Ordering::Relaxed) => ("cancelled", error),
            Err(error) => ("failed", error),
        };
        finish(&workbench, &task_id, status, &error);
        emit_status(&app, &task_id, "workflow", status, &error);
        if let Ok(mut tasks) = workbench.cancellations.lock() {
            tasks.remove(&task_id);
        }
        crate::task_lifecycle::publish_workbench_completion(
            &app,
            crate::task_lifecycle::WorkbenchTaskCompletion {
                task_id,
                device_id,
                kind: "workflow".to_owned(),
                status: status.to_owned(),
                error,
            },
        );
    });
    Ok(returned)
}

#[allow(clippy::too_many_arguments)]
fn run(
    app: &AppHandle,
    state: &WorkbenchState,
    serial: &SerialState,
    task_id: &str,
    host: &super::types::HostConfig,
    project: &ProjectConfig,
    device: &DeviceConfig,
    credentials: &SshCredentials,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    step(state, app, task_id, "preflight", || {
        check_cancel(cancellation)?;
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::GetState,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        if device.flasher_kind != "k230" {
            flasher::resolve_executable(app, &device.flasher_kind, &device.flasher_path)?;
        }
        ssh::connect(host, credentials)?;
        Ok(())
    })?;

    let before = remote_artifact_mtime(host, project, credentials).ok();
    step(state, app, task_id, "build", || {
        let audit = commands::begin_remote_task_audit(
            app,
            task_id,
            "workflow_build",
            commands::RemoteTaskActor::User,
            host,
            Some(project),
            serde_json::json!({"buildCommandByteCount": project.build_command.len()}),
        )?;
        let result = commands::run_build_task(
            app,
            task_id,
            host,
            project,
            credentials,
            cancellation,
            &audit,
        );
        let audit_status = match &result {
            Ok(0) => "success",
            Ok(_) | Err(_) => "failed",
        };
        commands::finish_remote_task_audit(app, &audit, audit_status, &result);
        let exit = result?;
        if exit == 0 {
            Ok(())
        } else {
            Err(format!("BUILD_EXIT:{exit}"))
        }
    })?;
    let after = remote_artifact_mtime(host, project, credentials)?;
    if before.is_some_and(|value| value == after) {
        return Err("STALE_FIRMWARE:artifact was not updated by this build".to_owned());
    }

    let mut download_project = project.clone();
    download_project.local_download_dir = PathBuf::from(&project.local_download_dir)
        .join(task_id)
        .to_string_lossy()
        .into_owned();
    let firmware = step_value(state, app, task_id, "transfer", || {
        let audit = commands::begin_remote_task_audit(
            app,
            task_id,
            "workflow_download",
            commands::RemoteTaskActor::User,
            host,
            Some(&download_project),
            serde_json::json!({
                "remotePathByteCount": download_project.firmware_path.len(),
                "verifySha256": download_project.verify_sha256,
            }),
        )?;
        let result = commands::download_task(
            app,
            task_id,
            host,
            &download_project,
            credentials,
            cancellation,
            &audit,
        );
        commands::finish_remote_task_audit(
            app,
            &audit,
            if result.is_ok() { "success" } else { "failed" },
            &result,
        );
        result
    })?;

    step(state, app, task_id, "serial_release", || {
        release_serial(serial, device)
    })?;
    step(state, app, task_id, "enter_flash_mode", || {
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::PowerOff,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        sleep_cancel(device.power_off_wait_ms, cancellation)?;
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::BootAssert,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::PowerOn,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        sleep_cancel(device.power_on_wait_ms, cancellation)
    })?;

    step(state, app, task_id, "flash", || {
        let firmware_path = firmware.to_string_lossy();
        flasher::publish_flash_intent(
            app,
            task_id,
            device,
            &firmware_path,
            flasher::FlashAuditActor::Workflow,
        )?;
        let result = (|| {
            let flasher_lock =
                state.flasher_target_lock(&device.flasher_kind, &device.usb_binding)?;
            let _flasher_guard = flasher_lock.try_lock().map_err(|_| {
                format!(
                    "FLASHER_BUSY:{} is currently performing a hardware operation",
                    device.flasher_kind
                )
            })?;
            let mut args = flasher::parse_extra_args(&device.flasher_args)?;
            if device.flasher_kind == "k230" {
                let exit = flasher::run_k230_flasher(
                    app,
                    task_id,
                    device,
                    &firmware_path,
                    &args,
                    cancellation,
                )?;
                return if exit == 0 {
                    Ok(())
                } else {
                    Err(format!("FLASH_EXIT:{exit}"))
                };
            }
            let executable =
                flasher::resolve_executable(app, &device.flasher_kind, &device.flasher_path)?;
            if args.is_empty() && device.flasher_kind != "uuu" {
                args = vec![
                    "--verify".into(),
                    "true".into(),
                    "--mode".into(),
                    "full_erase".into(),
                    "--post-action".into(),
                    "poweroff".into(),
                ];
            }
            let exit = flasher::run_flasher(
                app,
                task_id,
                device,
                &firmware_path,
                args,
                cancellation,
                &executable,
                Some(state),
            )?;
            if exit == 0 {
                Ok(())
            } else {
                Err(format!("FLASH_EXIT:{exit}"))
            }
        })();
        let (status, error) = match &result {
            Ok(()) => ("success", ""),
            Err(error) => ("failed", error.as_str()),
        };
        flasher::publish_flash_result(
            app,
            task_id,
            device,
            status,
            None,
            flasher::FlashAuditActor::Workflow,
            error,
        )?;
        result
    })?;

    step(state, app, task_id, "restart", || {
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::PowerOff,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        sleep_cancel(device.power_off_wait_ms, cancellation)?;
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::BootRelease,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        power::execute_and_audit(
            app,
            state,
            Some(task_id),
            device,
            PowerAction::PowerOn,
            0,
            power::PowerAuditActor::Workflow,
        )?;
        sleep_cancel(device.power_on_wait_ms, cancellation)
    })?;
    step(state, app, task_id, "serial_validate", || {
        reconnect_and_validate(app, serial, task_id, device, &firmware, cancellation)
    })
}

fn remote_artifact_mtime(
    host: &super::types::HostConfig,
    project: &ProjectConfig,
    credentials: &SshCredentials,
) -> Result<u64, String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_sftp_stat");
    let (session, _, _) = ssh::connect(host, credentials)?;
    let remote_path = commands::resolve_remote_artifact(&session, &project.firmware_path)?;
    let sftp = session.sftp().map_err(|e| format!("SFTP_OPEN:{e}"))?;
    sftp.stat(std::path::Path::new(&remote_path))
        .map_err(|e| format!("FIRMWARE_NOT_FOUND:{remote_path}:{e}"))?
        .mtime
        .ok_or_else(|| "FIRMWARE_MTIME_UNAVAILABLE".to_owned())
}

fn release_serial(serial: &SerialState, device: &DeviceConfig) -> Result<(), String> {
    let identity = serial_identity(device);
    match resolve_port_core(&identity) {
        Ok(port) if serial_is_open_core(serial, &port.name) => {
            serial_close_core(serial, &port.name)
        }
        Ok(_) => Ok(()),
        Err(error) if error.starts_with("SERIAL_NOT_FOUND:") => Ok(()),
        Err(error) => Err(error),
    }
}

fn reconnect_and_validate(
    app: &AppHandle,
    serial: &SerialState,
    task_id: &str,
    device: &DeviceConfig,
    firmware: &std::path::Path,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    let deadline = Instant::now()
        + Duration::from_millis(device.serial_reconnect_timeout_ms.clamp(100, 120_000));
    let resolved = loop {
        check_cancel(cancellation)?;
        match resolve_port_core(&serial_identity(device)) {
            Ok(port) => break port,
            Err(error) if error.starts_with("SERIAL_NOT_FOUND:") && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(format!("SERIAL_ENUM_TIMEOUT:{error}")),
        }
    };
    serial_open_core(
        serial,
        app.clone(),
        resolved.name.clone(),
        device.baud_rate,
        8,
        1,
        "none".into(),
        "none".into(),
        None,
    )?;
    let log_path = firmware
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("boot.log");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("SERIAL_LOG:{e}"))?;
    writeln!(
        log,
        "# task_id={task_id} device={} session_started={}",
        device.name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
    .map_err(|e| format!("SERIAL_LOG:{e}"))?;
    let deadline =
        Instant::now() + Duration::from_millis(device.boot_timeout_ms.clamp(1000, 300_000));
    let mut collected = String::new();
    while Instant::now() < deadline {
        check_cancel(cancellation)?;
        let bytes = serial_read_core(serial, &resolved.name, 8192)?;
        if !bytes.is_empty() {
            log.write_all(&bytes)
                .map_err(|e| format!("SERIAL_LOG:{e}"))?;
            collected.push_str(&String::from_utf8_lossy(&bytes));
            if matches!(boot_signal(&collected), BootSignal::KernelPanic) {
                return Err(format!("BOOT_KERNEL_PANIC:{}", log_path.display()));
            }
            if matches!(boot_signal(&collected), BootSignal::Ready) {
                return Ok(());
            }
            if collected.len() > 128 * 1024 {
                collected.drain(..64 * 1024);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("BOOT_TIMEOUT:{}", log_path.display()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootSignal {
    Pending,
    Ready,
    KernelPanic,
}

fn boot_signal(log: &str) -> BootSignal {
    if log.contains("Kernel panic") {
        BootSignal::KernelPanic
    } else if log.contains("login:") {
        BootSignal::Ready
    } else {
        BootSignal::Pending
    }
}

fn serial_identity(device: &DeviceConfig) -> SerialIdentity {
    SerialIdentity {
        port: device.serial_port.clone(),
        vid: device.serial_vid,
        pid: device.serial_pid,
        serial_number: device.serial_number.clone(),
        interface_number: device.serial_interface_number,
    }
}

fn step<F>(
    state: &WorkbenchState,
    app: &AppHandle,
    task_id: &str,
    name: &str,
    operation: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    step_value(state, app, task_id, name, operation)
}

fn step_value<T, F>(
    state: &WorkbenchState,
    app: &AppHandle,
    task_id: &str,
    name: &str,
    operation: F,
) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String>,
{
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "INSERT INTO workflow_steps(task_id,step_name,status,started_at) VALUES(?1,?2,'running',unixepoch())",
            rusqlite::params![task_id, name],
        );
    }
    emit_status(app, task_id, name, "running", name);
    let result = operation();
    let (status, error) = match &result {
        Ok(_) => ("success", ""),
        Err(error) => ("failed", error.as_str()),
    };
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "UPDATE workflow_steps SET status=?1,finished_at=unixepoch(),error=?2 \
             WHERE id=(SELECT id FROM workflow_steps WHERE task_id=?3 AND step_name=?4 ORDER BY id DESC LIMIT 1)",
            rusqlite::params![status, error, task_id, name],
        );
    }
    let message = if error.is_empty() {
        format!("{name}: {status}")
    } else {
        format!("{name}: {status}: {error}")
    };
    // Step completion is not task completion. Keep the event status running so
    // the UI retains cancellation controls until the final workflow event.
    emit_status(app, task_id, name, "running", &message);
    result
}

fn sleep_cancel(milliseconds: u64, cancellation: &AtomicBool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < deadline {
        check_cancel(cancellation)?;
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn check_cancel(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Relaxed) {
        Err("Task cancelled by user".to_owned())
    } else {
        Ok(())
    }
}

fn finish(state: &WorkbenchState, task_id: &str, status: &str, error: &str) {
    if let Ok(db) = state.db.lock() {
        let _ = super::storage::transition_task_status(&db, task_id, status, None, error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_validation_detects_ready_prompt() {
        assert_eq!(
            boot_signal("U-Boot\nStarting kernel\nboard login:"),
            BootSignal::Ready
        );
    }

    #[test]
    fn kernel_panic_wins_over_login_text() {
        assert_eq!(
            boot_signal("old login:\nKernel panic - not syncing"),
            BootSignal::KernelPanic
        );
    }

    #[test]
    fn boot_validation_waits_for_decisive_marker() {
        assert_eq!(boot_signal("Starting kernel ..."), BootSignal::Pending);
    }
}

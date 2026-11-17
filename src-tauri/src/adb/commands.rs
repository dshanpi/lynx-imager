use log::{debug, error};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

use super::manager::AdbManager;
use super::types::{AdbDevice, AdbDirectoryListing, AdbFileInfo, AdbServerStatus};
use crate::app_error::AppError;

pub struct AdbState(pub Mutex<AdbManager>);

#[derive(Debug)]
struct BoundAdbBoard {
    device_id: i64,
    serial: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AdbOperationActor {
    User,
    Gateway,
}

impl AdbOperationActor {
    fn identity(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user", "local-user"),
            Self::Gateway => ("ai", "linux-mcp-gateway"),
        }
    }
}

fn bound_adb_board(app: &AppHandle, serial: String) -> Result<BoundAdbBoard, AppError> {
    let workbench = app.state::<crate::workbench::WorkbenchState>();
    let db = workbench
        .db
        .lock()
        .map_err(|_| AppError::internal("WORKBENCH_DATABASE_LOCK_FAILED"))?;
    let matches = crate::workbench::storage::list_devices(&db)
        .map_err(|_| AppError::internal("WORKBENCH_DEVICE_LOOKUP_FAILED"))?
        .into_iter()
        .filter(|device| device.adb_serial.trim() == serial)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [device] => Ok(BoundAdbBoard {
            device_id: device
                .id
                .ok_or_else(|| AppError::internal("WORKBENCH_DEVICE_ID_MISSING"))?,
            serial,
        }),
        [] => Err(AppError::from("ADB_DEVICE_NOT_BOUND_TO_LINUX_BOARD")),
        _ => Err(AppError::from("ADB_DEVICE_BINDING_CONFLICT")),
    }
}

fn publish_adb_operation_intent(
    app: &AppHandle,
    board: &BoundAdbBoard,
    operation: &str,
    actor: AdbOperationActor,
    metadata: serde_json::Value,
) -> Result<String, AppError> {
    let (actor_type, actor_id) = actor.identity();
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "adb".to_owned(),
            action: format!("adb_{operation}_intended"),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", board.device_id),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: None,
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: format!("准备执行受控 ADB {operation} 操作"),
            metadata,
        },
    )
    .map_err(AppError::from)?;
    match outcome {
        crate::audit::AuditWriteOutcome::Recorded { event_id } => Ok(event_id),
        crate::audit::AuditWriteOutcome::Degraded { .. } => {
            Err(AppError::from("AUDIT_REQUIRED_DEGRADED"))
        }
    }
}

fn publish_adb_operation_result<T>(
    app: &AppHandle,
    board: &BoundAdbBoard,
    operation: &str,
    actor: AdbOperationActor,
    causation_id: &str,
    result: &Result<T, String>,
) {
    let (actor_type, actor_id) = actor.identity();
    let error_code = result
        .as_ref()
        .err()
        .map(|error| crate::audit::machine_error_code(error, "ADB_OPERATION_FAILED"));
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "adb".to_owned(),
            action: format!("adb_{operation}_completed"),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", board.device_id),
            session_id: None,
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: None,
            causation_id: Some(causation_id.to_owned()),
            result: if result.is_ok() { "success" } else { "failed" }.to_owned(),
            error_code,
            summary: format!(
                "ADB {operation} 操作{}",
                if result.is_ok() { "完成" } else { "失败" }
            ),
            metadata: serde_json::json!({}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn board_operation_lock(
    app: &AppHandle,
    device_id: i64,
) -> Result<std::sync::Arc<std::sync::Mutex<()>>, AppError> {
    app.state::<crate::workbench::WorkbenchState>()
        .device_lock(device_id)
        .map_err(AppError::from)
}

fn execute_bound_adb_operation<T>(
    app: &AppHandle,
    state: &AdbState,
    requested_serial: Option<&str>,
    operation: &str,
    actor: AdbOperationActor,
    metadata: serde_json::Value,
    execute: impl FnOnce(&AdbManager, &str) -> Result<T, String>,
) -> Result<T, AppError> {
    let serial = match requested_serial
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(serial) => serial.to_owned(),
        None => state
            .0
            .lock()
            .map_err(|_| AppError::internal("ADB_MANAGER_LOCK_FAILED"))?
            .get_selected_device()
            .ok_or_else(|| AppError::from("ADB_DEVICE_SERIAL_REQUIRED"))?,
    };
    let board = bound_adb_board(app, serial)?;
    let device_lock = board_operation_lock(app, board.device_id)?;
    let _device_guard = device_lock
        .try_lock()
        .map_err(|_| AppError::from("DEVICE_BUSY"))?;
    let intent_id = publish_adb_operation_intent(app, &board, operation, actor, metadata)?;
    let manager = state
        .0
        .lock()
        .map_err(|_| AppError::internal("ADB_MANAGER_LOCK_FAILED"))?;
    let result = execute(&manager, &board.serial);
    publish_adb_operation_result(app, &board, operation, actor, &intent_id, &result);
    result.map_err(AppError::from)
}

#[tauri::command]
pub async fn adb_check_server(state: State<'_, AdbState>) -> Result<AdbServerStatus, AppError> {
    debug!("adb_check_server called");

    let addr = {
        let manager = state.0.lock().map_err(|e| {
            error!("Failed to lock AdbManager: {}", e);
            AppError::internal(e.to_string())
        })?;
        manager.get_server_addr()
    };

    debug!("Checking server status at address: {}", addr);

    tokio::task::spawn_blocking(move || AdbManager::check_server_status_with_addr(addr))
        .await
        .map_err(|e| {
            error!("Task join error: {}", e);
            AppError::internal(e.to_string())
        })
}

#[tauri::command]
pub async fn adb_list_devices(
    app: AppHandle,
    state: State<'_, AdbState>,
) -> Result<Vec<AdbDevice>, AppError> {
    debug!("adb_list_devices called");

    crate::hardware_safety::ensure_idle(&app, "扫描 ADB USB 设备").map_err(AppError::from)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("ADB", "USB 设备扫描", "adb")
        .map_err(AppError::from)?;

    let manager = state
        .0
        .lock()
        .map_err(|e| {
            error!("Failed to lock AdbManager: {}", e);
            AppError::internal(e.to_string())
        })?
        .clone();

    tokio::task::spawn_blocking(move || manager.list_devices())
        .await
        .map_err(|e| {
            error!("Task join error: {}", e);
            AppError::internal(e.to_string())
        })?
        .map_err(AppError::from)
}

#[tauri::command]
pub fn adb_select_device(state: State<AdbState>, serial: String) -> Result<(), AppError> {
    debug!("adb_select_device called: serial={}", serial);

    let manager = state.0.lock().map_err(|e| {
        error!("Failed to lock AdbManager: {}", e);
        AppError::internal(e.to_string())
    })?;
    manager.select_device(serial);
    Ok(())
}

#[tauri::command]
pub fn adb_get_selected_device(state: State<AdbState>) -> Option<String> {
    debug!("adb_get_selected_device called");

    let manager = state.0.lock().ok()?;
    let result = manager.get_selected_device();

    if let Some(ref serial) = result {
        debug!("Selected device: {}", serial);
    } else {
        debug!("No device selected");
    }

    result
}

#[tauri::command]
pub fn adb_clear_selected_device(state: State<AdbState>) {
    debug!("adb_clear_selected_device called");

    let manager = state.0.lock().unwrap();
    manager.clear_selected_device();
}

#[tauri::command]
pub fn adb_shell_command(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    command: String,
) -> Result<String, AppError> {
    debug!(
        "adb_shell_command called: serial={:?}, command_bytes={}",
        serial,
        command.len()
    );

    adb_shell_command_core(&app, &state, serial, command, AdbOperationActor::User)
}

pub(crate) fn adb_shell_command_core(
    app: &AppHandle,
    state: &AdbState,
    serial: Option<String>,
    command: String,
    actor: AdbOperationActor,
) -> Result<String, AppError> {
    execute_bound_adb_operation(
        app,
        state,
        serial.as_deref(),
        "shell_command",
        actor,
        serde_json::json!({"commandByteCount": command.len()}),
        |manager, serial| manager.shell_command(Some(serial), &command),
    )
}

/// Fixed, bounded recipe used only after the Gateway acquires its hardware guard.
pub(crate) fn adb_rockchip_enter_loader_core(
    app: &AppHandle,
    device_id: i64,
    serial: &str,
) -> Result<String, AppError> {
    let board = bound_adb_board(app, serial.to_owned())?;
    if board.device_id != device_id {
        return Err(AppError::from("ROCKCHIP_LOADER_SCOPE_CHANGED"));
    }
    execute_bound_adb_operation(
        app,
        &app.state::<AdbState>(),
        Some(serial),
        "rockchip_enter_loader",
        AdbOperationActor::Gateway,
        serde_json::json!({"recipe": "adb_reboot_loader"}),
        |manager, bound_serial| {
            manager.shell_command_with_timeout(
                Some(bound_serial),
                "reboot loader",
                std::time::Duration::from_secs(5),
                4096,
            )
        },
    )
}

#[tauri::command]
pub fn adb_list_directory(
    state: State<AdbState>,
    serial: Option<String>,
    path: String,
) -> Result<AdbDirectoryListing, AppError> {
    debug!(
        "adb_list_directory called: serial={:?}, path_bytes={}",
        serial,
        path.len()
    );

    let manager = state.0.lock().map_err(|e| {
        error!("Failed to lock AdbManager: {}", e);
        AppError::internal(e.to_string())
    })?;
    let serial_ref = serial.as_deref();
    let result = manager.list_directory(serial_ref, &path);

    if let Ok(ref listing) = result {
        debug!("Listed directory: {} items found", listing.items.len());
    }

    result.map_err(AppError::from)
}

#[tauri::command]
pub fn adb_push_file(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    local_path: String,
    remote_path: String,
) -> Result<(), AppError> {
    debug!(
        "adb_push_file called: serial={:?}, local_path_bytes={}, remote_path_bytes={}",
        serial,
        local_path.len(),
        remote_path.len()
    );
    let file_bytes = std::fs::metadata(&local_path).map(|meta| meta.len()).ok();
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "push_file",
        AdbOperationActor::User,
        serde_json::json!({
            "localPathByteCount": local_path.len(),
            "remotePathByteCount": remote_path.len(),
            "fileByteCount": file_bytes,
        }),
        |manager, serial| manager.push_file(Some(serial), &local_path, &remote_path),
    )
}

#[tauri::command]
pub fn adb_pull_file(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    remote_path: String,
    local_path: String,
) -> Result<(), AppError> {
    debug!(
        "adb_pull_file called: serial={:?}, remote_path_bytes={}, local_path_bytes={}",
        serial,
        remote_path.len(),
        local_path.len()
    );
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "pull_file",
        AdbOperationActor::User,
        serde_json::json!({
            "remotePathByteCount": remote_path.len(),
            "localPathByteCount": local_path.len(),
        }),
        |manager, serial| manager.pull_file(Some(serial), &remote_path, &local_path),
    )
}

#[tauri::command]
pub fn adb_pull_folder(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    remote_path: String,
    local_path: String,
) -> Result<(), AppError> {
    debug!(
        "adb_pull_folder called: serial={:?}, remote_path_bytes={}, local_path_bytes={}",
        serial,
        remote_path.len(),
        local_path.len()
    );
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "pull_folder",
        AdbOperationActor::User,
        serde_json::json!({
            "remotePathByteCount": remote_path.len(),
            "localPathByteCount": local_path.len(),
        }),
        |manager, serial| manager.pull_folder(Some(serial), &remote_path, &local_path),
    )
}

#[tauri::command]
pub fn adb_delete_file(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    path: String,
) -> Result<String, AppError> {
    debug!(
        "adb_delete_file called: serial={:?}, path_bytes={}",
        serial,
        path.len()
    );
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "delete_file",
        AdbOperationActor::User,
        serde_json::json!({"pathByteCount": path.len()}),
        |manager, serial| manager.delete_file(Some(serial), &path),
    )
}

#[tauri::command]
pub fn adb_make_directory(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    path: String,
) -> Result<String, AppError> {
    debug!(
        "adb_make_directory called: serial={:?}, path_bytes={}",
        serial,
        path.len()
    );
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "make_directory",
        AdbOperationActor::User,
        serde_json::json!({"pathByteCount": path.len()}),
        |manager, serial| manager.make_directory(Some(serial), &path),
    )
}

#[tauri::command]
pub fn adb_rename(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    old_path: String,
    new_path: String,
) -> Result<String, AppError> {
    debug!(
        "adb_rename called: serial={:?}, old_path_bytes={}, new_path_bytes={}",
        serial,
        old_path.len(),
        new_path.len()
    );
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "rename",
        AdbOperationActor::User,
        serde_json::json!({
            "oldPathByteCount": old_path.len(),
            "newPathByteCount": new_path.len(),
        }),
        |manager, serial| manager.rename(Some(serial), &old_path, &new_path),
    )
}

#[tauri::command]
pub fn adb_stat(
    state: State<AdbState>,
    serial: Option<String>,
    path: String,
) -> Result<AdbFileInfo, AppError> {
    debug!(
        "adb_stat called: serial={:?}, path_bytes={}",
        serial,
        path.len()
    );

    let manager = state.0.lock().map_err(|e| {
        error!("Failed to lock AdbManager: {}", e);
        AppError::internal(e.to_string())
    })?;
    let serial_ref = serial.as_deref();
    manager.stat(serial_ref, &path).map_err(AppError::from)
}

#[tauri::command]
pub fn adb_reboot(
    app: tauri::AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
    reboot_type: String,
) -> Result<(), AppError> {
    let reboot_type = normalize_reboot_type(&reboot_type).map_err(AppError::from)?;
    crate::hardware_safety::ensure_idle(&app, "ADB 重启").map_err(AppError::from)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("ADB", "重启", serial.clone().unwrap_or_default())
        .map_err(AppError::from)?;
    debug!(
        "adb_reboot called: serial={:?}, type={}",
        serial, reboot_type
    );

    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "reboot",
        AdbOperationActor::User,
        serde_json::json!({"rebootType": reboot_type}),
        |manager, serial| manager.reboot(Some(serial), reboot_type),
    )
}

#[tauri::command]
pub fn adb_root(
    app: AppHandle,
    state: State<AdbState>,
    serial: Option<String>,
) -> Result<String, AppError> {
    debug!("adb_root called: serial={:?}", serial);
    crate::hardware_safety::ensure_idle(&app, "ADB root").map_err(AppError::from)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("ADB", "root", serial.clone().unwrap_or_default())
        .map_err(AppError::from)?;
    execute_bound_adb_operation(
        &app,
        &state,
        serial.as_deref(),
        "root",
        AdbOperationActor::User,
        serde_json::json!({}),
        |manager, serial| manager.root(Some(serial)),
    )
}

fn normalize_reboot_type(value: &str) -> Result<&'static str, String> {
    match value.trim() {
        "" | "system" | "reboot" => Ok("system"),
        "bootloader" | "reboot bootloader" => Ok("bootloader"),
        "recovery" | "reboot recovery" => Ok("recovery"),
        "sideload" => Ok("sideload"),
        "sideload-auto-reboot" => Ok("sideload-auto-reboot"),
        "fastboot" => Ok("fastboot"),
        _ => Err("ADB_REBOOT_TYPE_INVALID".to_owned()),
    }
}

#[macro_export]
macro_rules! register_adb_commands {
    () => {
        $crate::adb::commands::adb_check_server,
        $crate::adb::commands::adb_list_devices,
        $crate::adb::commands::adb_select_device,
        $crate::adb::commands::adb_get_selected_device,
        $crate::adb::commands::adb_clear_selected_device,
        $crate::adb::commands::adb_shell_command,
        $crate::adb::commands::adb_list_directory,
        $crate::adb::commands::adb_push_file,
        $crate::adb::commands::adb_pull_file,
        $crate::adb::commands::adb_pull_folder,
        $crate::adb::commands::adb_delete_file,
        $crate::adb::commands::adb_make_directory,
        $crate::adb::commands::adb_rename,
        $crate::adb::commands::adb_stat,
        $crate::adb::commands::adb_reboot,
        $crate::adb::commands::adb_root
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reboot_type_is_explicit_and_never_falls_back_to_recovery() {
        assert_eq!(normalize_reboot_type("reboot").unwrap(), "system");
        assert_eq!(
            normalize_reboot_type("reboot bootloader").unwrap(),
            "bootloader"
        );
        assert_eq!(
            normalize_reboot_type("efex").unwrap_err(),
            "ADB_REBOOT_TYPE_INVALID"
        );
    }
}

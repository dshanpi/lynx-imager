use super::task;
use super::types::{FlashOptions, FlashStartResult};
use crate::app_error::AppError;
use tauri::{AppHandle, Manager, Runtime};

#[tauri::command]
pub fn flash_cancel(task_id: u64) -> Result<(), AppError> {
    task::cancel(task_id)
}

#[tauri::command]
pub fn flash_confirm(task_id: u64, request_id: u64, confirmed: bool) -> Result<(), AppError> {
    task::confirm(task_id, request_id, confirmed)
}

#[tauri::command]
pub fn flash_start<R: Runtime>(
    app_handle: AppHandle<R>,
    device_id: u32,
    bus: u8,
    port: u8,
    image_path: String,
    options: FlashOptions,
) -> Result<FlashStartResult, AppError> {
    crate::hardware_safety::ensure_idle(&app_handle, "通用烧录").map_err(AppError::from)?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let hardware_guard = safety
        .try_begin("通用烧录", "烧录", format!("usb:{bus}:{port}"))
        .map_err(AppError::from)?;
    task::start(
        app_handle,
        device_id,
        bus,
        port,
        image_path,
        options,
        Some(hardware_guard),
    )
}

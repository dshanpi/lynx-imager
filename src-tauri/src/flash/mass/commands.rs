use tauri::{AppHandle, Manager, Runtime, State};

use crate::app_error::AppError;
use crate::flash::types::FlashOptions;

use super::types::MassStatusSnapshot;
use super::MassProductionState;

#[tauri::command]
pub async fn mass_start<R: Runtime>(
    app_handle: AppHandle<R>,
    state: State<'_, MassProductionState>,
    image_path: String,
    options: FlashOptions,
    max_slots: u16,
) -> Result<(), AppError> {
    crate::hardware_safety::ensure_idle(&app_handle, "批量烧录").map_err(AppError::from)?;
    crate::flash_owner::check_start("openixcli").map_err(AppError::from)?;
    let safety = app_handle.state::<crate::hardware_safety::HardwareSafetyState>();
    let hardware_guard = safety
        .try_begin("批量烧录", "等待并烧录设备", format!("slots:{max_slots}"))
        .map_err(AppError::from)?;
    let mut manager = state.0.lock().unwrap();
    manager.start(&app_handle, image_path, options, max_slots, hardware_guard)
}

#[tauri::command]
pub fn mass_stop<R: Runtime>(
    app_handle: AppHandle<R>,
    state: State<'_, MassProductionState>,
) -> Result<(), AppError> {
    let mut manager = state.0.lock().unwrap();
    manager.stop(&app_handle)
}

#[tauri::command]
pub fn mass_get_status(
    state: State<'_, MassProductionState>,
) -> Result<MassStatusSnapshot, AppError> {
    let manager = state.0.lock().unwrap();
    Ok(manager.get_status())
}

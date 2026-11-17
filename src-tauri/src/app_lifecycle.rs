use crate::adb::commands::AdbState;
use crate::{
    adb, auth, ch347, cloud_sync, diagnostics, gateway, hardware_monitor, hardware_safety,
    learning, mcu, records, serial, supervisor, task_bar, tcp, workbench,
};
use std::sync::Arc;
use tauri::{Emitter, Manager};

pub fn manage_runtime_state(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        .manage(AdbState(Default::default()))
        .manage(Arc::new(adb::transfer::AdbTransferState::default()))
        .manage(crate::flash::mass::MassProductionState::new())
        // CH347 / serial state is process-wide and shared by native tools.
        .manage(Arc::new(ch347::Ch347State::new()))
        .manage(Arc::new(serial::SerialState::new()))
        .manage(tcp::TcpState::new())
        .manage(hardware_safety::HardwareSafetyState::default())
        .manage(Arc::new(hardware_monitor::HardwareMonitorState::default()))
        .manage(mcu::McuState::default())
        .manage(mcu::mcp::McuMcpGatewayState::default())
        .manage(mcu::mcp::McuMcpTunnelState::default())
        .manage(gateway::GatewayState::default())
        .manage(Arc::new(auth::state::AuthState::new()))
        .manage(learning::deep_link::LearningDeepLinkState::default())
        .manage(task_bar::TaskbarIndicatorState::new())
        .manage(crate::audit::AuditSinkState::default())
        .manage(crate::content_digest::ContentDigestState::default())
        .manage(crate::task_lifecycle::TaskLifecycleState::default())
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("LYNX_LIFECYCLE_SETUP_BEGIN");
    learning::deep_link::setup(app)?;
    task_bar::setup_taskbar_listeners(app.handle());
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(Box::<dyn std::error::Error>::from)?;
    let local_learning = learning::local::LocalLearningState::open(
        &data_dir.join("learning.sqlite"),
        data_dir.clone(),
    )
    .map_err(boxed_io_error)?;
    app.manage(Arc::new(local_learning));
    let panic_marker_path = data_dir.join("diagnostic-panic-marker.json");
    diagnostics::configure_panic_marker(panic_marker_path.clone());
    diagnostics::configure_native_operation_marker(
        data_dir.join("diagnostic-native-operation-marker.json"),
    );
    let diagnostics = diagnostics::DiagnosticState::open(
        &data_dir.join("diagnostics.sqlite"),
        &panic_marker_path,
    )
    .map_err(boxed_io_error)?;
    app.manage(Arc::new(diagnostics));
    let workbench = workbench::WorkbenchState::open(&data_dir.join("workbench.sqlite"))
        .map_err(|error| boxed_io_error(error.to_string()))?;
    let workbench = Arc::new(workbench);
    workbench::flasher::cleanup_rockchip_raw_cache(app.handle());
    let supervisor = supervisor::SupervisorState::open(&data_dir.join("supervisor.sqlite"))
        .map_err(|error| boxed_io_error(error.to_string()))?;
    {
        let workbench_db = workbench
            .db
            .lock()
            .map_err(|error| boxed_io_error(error.to_string()))?;
        let mut supervisor_db = supervisor
            .db
            .lock()
            .map_err(|error| boxed_io_error(error.to_string()))?;
        let synced = supervisor::storage::sync_board_identities(&mut supervisor_db, &workbench_db)
            .map_err(|error| boxed_io_error(error.to_string()))?;
        log::debug!("Synchronized {synced} Linux board identity mapping(s)");
    }
    let reconciled = supervisor::reconcile_interrupted_runs(&supervisor, &workbench)
        .map_err(|error| boxed_io_error(error.to_string()))?;
    if reconciled > 0 {
        log::info!("Reconciled {reconciled} interrupted supervisor run(s)");
    }
    app.manage(Arc::new(supervisor));
    let records = records::RecordCenterState::open(&data_dir.join("records.sqlite"))
        .map_err(|error| boxed_io_error(error.to_string()))?;
    {
        let workbench_db = workbench
            .db
            .lock()
            .map_err(|error| boxed_io_error(error.to_string()))?;
        let mut records_db = records
            .db
            .lock()
            .map_err(|error| boxed_io_error(error.to_string()))?;
        let synced = records::storage::sync_board_identities(&mut records_db, &workbench_db)
            .map_err(|error| boxed_io_error(error.to_string()))?;
        log::debug!("Synchronized {synced} Record Center board identity mapping(s)");
    }
    let records = Arc::new(records);
    app.manage(records.clone());
    let saved_projects = {
        let db = workbench
            .db
            .lock()
            .map_err(|error| boxed_io_error(error.to_string()))?;
        workbench::storage::list_projects(&db).map_err(|error| boxed_io_error(error.to_string()))?
    };
    app.manage(workbench.clone());
    let cloud_sync = cloud_sync::CloudSyncState::open(&data_dir.join("cloud-sync.sqlite"))
        .map_err(boxed_io_error)?;
    app.manage(Arc::new(cloud_sync));
    records::register_audit_sink(app.handle(), records, workbench.clone())
        .map_err(|error| boxed_io_error(error.to_string()))?;
    crate::supervisor::register_task_lifecycle_port(app.handle())
        .map_err(|error| boxed_io_error(error.to_string()))?;
    supervisor::scheduler::start(app.handle().clone());
    cloud_sync::start(app.handle().clone());
    diagnostics::start(app.handle().clone());
    hardware_monitor::start_default(app.handle().clone());
    restore_saved_sdk_channels(app.handle().clone(), saved_projects, workbench);
    // 启动 USB 设备插拔监听（事件驱动设备扫描，替代前端轮询）
    ch347::device_notifier::start(app.handle().clone());
    log::info!("LYNX_LIFECYCLE_SETUP_READY");
    Ok(())
}

fn restore_saved_sdk_channels(
    app_handle: tauri::AppHandle,
    saved_projects: Vec<workbench::types::ProjectConfig>,
    workbench: Arc<workbench::WorkbenchState>,
) {
    tauri::async_runtime::spawn(async move {
        for project in saved_projects {
            let (Some(project_id), Some(device_id)) = (project.id, project.device_id) else {
                continue;
            };
            log::info!(
                "Restoring saved SDK channel project={project_id}, device={device_id}, gateway={}, remote={}",
                project.local_gateway_port,
                project.remote_forward_port
            );
            let gateway_state = app_handle.state::<gateway::GatewayState>();
            match gateway::start_gateway(
                app_handle.clone(),
                gateway_state.inner(),
                Some(project.local_gateway_port),
                Some(project_id),
                Some(device_id),
            )
            .await
            {
                Ok(_) => {
                    if let Err(error) = workbench::tunnel::start_tunnel(project_id, &workbench) {
                        log::warn!("Failed to restore SDK tunnel project={project_id}: {error}");
                        if let Some(state) =
                            app_handle.try_state::<Arc<diagnostics::DiagnosticState>>()
                        {
                            diagnostics::capture_backend_error(
                                state.inner(),
                                "sdk_channel_restore",
                                "SDK_TUNNEL_RESTORE_FAILED",
                                "background_channel_restore",
                            );
                        }
                    }
                }
                Err(error) => {
                    log::warn!("Failed to restore Gateway project={project_id}: {error:?}");
                    if let Some(state) = app_handle.try_state::<Arc<diagnostics::DiagnosticState>>()
                    {
                        diagnostics::capture_backend_error(
                            state.inner(),
                            "sdk_channel_restore",
                            "SDK_GATEWAY_RESTORE_FAILED",
                            "background_channel_restore",
                        );
                    }
                }
            }
        }
    });
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        log::info!("LYNX_LIFECYCLE_WINDOW_CLOSE_REQUESTED");
        if let Some(monitor) = window
            .app_handle()
            .try_state::<Arc<hardware_monitor::HardwareMonitorState>>()
        {
            monitor.shutdown(window.app_handle());
        }
        let reasons = hardware_safety::active_reasons(window.app_handle());
        if !reasons.is_empty() {
            api.prevent_close();
            log::warn!(
                "Blocked LYNX window close while hardware task is active: {}",
                reasons.join("; ")
            );
            let _ = window.emit("hardware-close-blocked", reasons);
        } else {
            log::info!("LYNX_LIFECYCLE_WINDOW_CLOSE_ALLOWED");
        }
    }
}

pub fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    if let tauri::RunEvent::ExitRequested { api, .. } = event {
        log::info!("LYNX_LIFECYCLE_EXIT_REQUESTED");
        if let Some(monitor) = app.try_state::<Arc<hardware_monitor::HardwareMonitorState>>() {
            monitor.shutdown(app);
        }
        let reasons = hardware_safety::active_reasons(app);
        if !reasons.is_empty() {
            api.prevent_exit();
            log::warn!(
                "Blocked LYNX exit while hardware task is active: {}",
                reasons.join("; ")
            );
            let _ = app.emit("hardware-close-blocked", reasons);
        } else {
            log::info!("LYNX_LIFECYCLE_EXIT_ALLOWED");
        }
    } else if let tauri::RunEvent::Exit = event {
        log::info!("LYNX_LIFECYCLE_EXIT_CLEANUP");
        let workbench = app.state::<Arc<workbench::WorkbenchState>>();
        workbench::ssh_terminal::shutdown_all(workbench.inner());
        log::info!("LYNX_LIFECYCLE_EXIT_COMPLETE");
    }
}

fn boxed_io_error(message: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::other(message))
}

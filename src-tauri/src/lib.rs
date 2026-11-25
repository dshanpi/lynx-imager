mod adb;
mod app_error;
mod app_lifecycle;
mod audit;
mod auth;
mod ch347;
mod cloud_sync;
mod config_audit;
mod content_digest;
mod database_migrations;
mod diagnostics;
mod disasm;
mod diskpart;
mod dtb;
mod efex;
mod file;
mod firmware;
mod flash;
mod flash_owner;
mod gateway;
mod hardware_monitor;
mod hardware_safety;
mod hotplug;
mod imager_card;
pub use imager_card::entry as card_worker_entry;
mod imager_board_loader;
mod imager_drivers;
mod imager_emmc;
mod imager_f101_xip;
mod imager_inspect;
mod imager_layout;
mod invoke_handler;
mod k230;
mod learning;
mod mcu;
mod packer;
mod process;
mod proxy;
mod records;
mod serial;
mod skills;
mod supervisor;
mod task_bar;
mod task_lifecycle;
mod tcp;
mod update_recovery;
mod usb;
mod workbench;

pub use app_error::IpcErrorContract;

pub(crate) fn keyring_service(service: &str) -> std::borrow::Cow<'_, str> {
    if option_env!("VITE_LYNX_LOCAL_BUILD") == Some("1") {
        format!("{service}.imager-local").into()
    } else {
        format!("{service}.imager").into()
    }
}

use tauri_plugin_log::{Target, TargetKind};

pub(crate) fn isolated_test_instance() -> bool {
    cfg!(debug_assertions)
        && std::env::var("LYNX_TEST_ALLOW_PARALLEL_INSTANCE").as_deref() == Ok("1")
}

#[tauri::command]
fn open_devtools(window: tauri::WebviewWindow) {
    window.open_devtools();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|panic_info| {
        let fingerprint = crate::diagnostics::record_panic_marker(panic_info.location());
        // Keep the historical `location=` marker for lifecycle monitoring while
        // deliberately withholding the local source path from diagnostics.
        eprintln!("LYNX_FATAL_PANIC location=redacted fingerprint={fingerprint}");
        log::error!("LYNX_FATAL_PANIC location=redacted fingerprint={fingerprint}");
    }));
    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        if !isolated_test_instance() {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
                use tauri::Manager;
                crate::learning::deep_link::accept_process_arguments(app, argv);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }));
        }
    }
    let builder = builder
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Warn
                })
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    app_lifecycle::manage_runtime_state(builder)
        .setup(app_lifecycle::setup)
        .invoke_handler(invoke_handler::handler())
        .on_window_event(app_lifecycle::handle_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(app_lifecycle::handle_run_event);
}

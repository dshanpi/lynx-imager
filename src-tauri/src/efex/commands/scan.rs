use log::{debug, warn};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use super::service::run_blocking_with_timeout;
use crate::efex::device;
use crate::efex::error::EfexError;
use crate::efex::types::{DeviceMode, EfexDevice};

// libusb device discovery cannot be cancelled after spawn_blocking starts.
// Serialize scans so a slow VMware USB transaction cannot overlap with the
// next UI/MCP poll and make both callers observe a busy/missing device.
static SCAN_LOCK: Mutex<()> = Mutex::const_new(());
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);

#[tauri::command]
pub async fn efex_scan_devices(app: AppHandle) -> Result<Vec<EfexDevice>, EfexError> {
    crate::hardware_safety::ensure_idle(&app, "EFEX USB 设备扫描").map_err(hardware_busy)?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety
        .try_begin("EFEX", "USB 设备扫描", "legacy-efex")
        .map_err(hardware_busy)?;
    efex_scan_devices_core().await
}

pub(crate) async fn efex_scan_devices_core() -> Result<Vec<EfexDevice>, EfexError> {
    let _scan_guard = SCAN_LOCK.lock().await;
    debug!("efex_scan_devices called");
    run_blocking_with_timeout(SCAN_TIMEOUT, "Scan device timeout", || {
        debug!("Scanning USB devices");
        let scanned = libefex::Context::scan_usb_devices()
            .map_err(|e| EfexError::from_libefex_with_device_id(&e, None))?;

        if scanned.is_empty() {
            debug!("No devices found");
            return Ok(Vec::new());
        }

        debug!("Found {} potential devices", scanned.len());

        let mut devices = Vec::new();
        for dev in scanned {
            let mut ctx = libefex::Context::new();

            if let Err(error) = ctx
                .scan_usb_device_at(dev.bus, dev.port)
                .map_err(|e| EfexError::from_libefex_with_device_id(&e, None))
            {
                warn!(
                    "Skipping device during scan: bus={}, port={}, step=scan, error={}",
                    dev.bus, dev.port, error.message
                );
                continue;
            }
            if let Err(error) = ctx
                .usb_init()
                .map_err(|e| EfexError::from_libefex_with_device_id(&e, None))
            {
                warn!(
                    "Skipping device during scan: bus={}, port={}, step=usb_init, error={}",
                    dev.bus, dev.port, error.message
                );
                continue;
            }
            if let Err(error) = ctx
                .efex_init()
                .map_err(|e| EfexError::from_libefex_with_device_id(&e, None))
            {
                warn!(
                    "Skipping device during scan: bus={}, port={}, step=efex_init, error={}",
                    dev.bus, dev.port, error.message
                );
                continue;
            }

            let mode: DeviceMode = ctx.get_device_mode().into();
            let mode_str = ctx.get_device_mode_str().to_string();
            let chip_version = unsafe { (*ctx.as_ptr()).resp.id };

            debug!(
                "Device found: bus={}, port={}, mode={:?}, chip=0x{:x}",
                dev.bus, dev.port, mode, chip_version
            );

            let device_id = device::register_device(
                dev.bus,
                dev.port,
                ctx.device_path().map(ToOwned::to_owned),
            );
            drop(ctx);

            devices.push(EfexDevice {
                device_id,
                chip_version,
                mode: mode.as_str().to_string(),
                mode_str,
                bus: dev.bus,
                port: dev.port,
            });
        }

        debug!("Returning {} devices", devices.len());
        Ok(devices)
    })
    .await
}

fn hardware_busy(message: String) -> EfexError {
    EfexError {
        code: -33,
        name: "DeviceBusy".to_owned(),
        message,
    }
}

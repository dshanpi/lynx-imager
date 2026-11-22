use log::debug;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use super::error::EfexError;

#[derive(Debug, Clone)]
struct RegisteredDevice {
    bus: u8,
    port: u8,
    device_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegisteredDeviceInfo {
    pub device_id: u32,
    pub port: u8,
    pub device_path: Option<String>,
}

lazy_static::lazy_static! {
    static ref DEVICE_MAP: Mutex<HashMap<u32, RegisteredDevice>> = Mutex::new(HashMap::new());
}

static NEXT_DEVICE_ID: AtomicU32 = AtomicU32::new(1);

fn invalid_device_error(device_id: u32) -> EfexError {
    EfexError {
        code: -101,
        name: "InvalidDeviceId".to_string(),
        message: format!("No EFEX device registered for deviceId {}", device_id),
    }
}

fn same_device(existing: &RegisteredDevice, bus: u8, port: u8, device_path: Option<&str>) -> bool {
    match (existing.device_path.as_deref(), device_path) {
        (Some(existing_path), Some(candidate_path)) => existing_path == candidate_path,
        _ => existing.bus == bus && existing.port == port,
    }
}

pub fn register_device(bus: u8, port: u8, device_path: Option<String>) -> u32 {
    let mut map = DEVICE_MAP.lock().unwrap();
    if let Some((device_id, existing)) = map
        .iter_mut()
        .find(|(_, existing)| same_device(existing, bus, port, device_path.as_deref()))
    {
        existing.bus = bus;
        existing.port = port;
        existing.device_path = device_path.clone();
        debug!(
            "Reusing registered device: device_id={}, bus={}, port={}, device_path={:?}",
            device_id, bus, port, device_path
        );
        return *device_id;
    }

    let device_id = NEXT_DEVICE_ID.fetch_add(1, Ordering::SeqCst);
    debug!(
        "Registering device: device_id={}, bus={}, port={}, device_path={:?}",
        device_id, bus, port, device_path
    );
    map.insert(
        device_id,
        RegisteredDevice {
            bus,
            port,
            device_path,
        },
    );
    device_id
}

pub fn unregister_device(device_id: u32) {
    debug!("Unregistering device: device_id={}", device_id);
    let mut map = DEVICE_MAP.lock().unwrap();
    map.remove(&device_id);
}

pub fn find_registered_device(
    device_path: Option<&str>,
    bus: Option<u8>,
    port: Option<u8>,
) -> Option<RegisteredDeviceInfo> {
    let map = DEVICE_MAP.lock().unwrap();
    map.iter().find_map(|(device_id, existing)| {
        let matches = match (device_path, bus, port) {
            (Some(path), _, _) => existing
                .device_path
                .as_deref()
                .is_some_and(|existing_path| existing_path.eq_ignore_ascii_case(path)),
            (None, Some(bus), Some(port)) => existing.bus == bus && existing.port == port,
            _ => false,
        };

        matches.then(|| RegisteredDeviceInfo {
            device_id: *device_id,
            port: existing.port,
            device_path: existing.device_path.clone(),
        })
    })
}

pub fn get_device_path(device_id: u32) -> Option<String> {
    let map = DEVICE_MAP.lock().unwrap();
    map.get(&device_id).and_then(|d| d.device_path.clone())
}

pub fn create_context_for(device_id: u32) -> Result<libefex::Context, EfexError> {
    let device = {
        let map = DEVICE_MAP.lock().unwrap();
        map.get(&device_id).cloned()
    }
    .ok_or_else(|| invalid_device_error(device_id))?;

    let addr = match device.device_path {
        Some(path) => crate::usb::DeviceAddress::parse(&path),
        None => crate::usb::DeviceAddress::from_bus_port(device.bus, device.port),
    };

    crate::usb::open_context(&addr).map_err(EfexError::from)
}

pub fn get_context(device_id: u32) -> Result<libefex::Context, EfexError> {
    crate::flash_owner::check_start("openixcli").map_err(EfexError::hardware_busy)?;
    create_context_for(device_id)
}

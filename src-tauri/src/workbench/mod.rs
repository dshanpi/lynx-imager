//! Remote embedded-development workbench.
//!
//! This module owns persisted host/project/device configuration, SSH/SFTP
//! operations, and task history. Hardware-specific flashing and power control
//! remain behind their existing adapters so the workflow does not depend on a
//! single chip vendor or controller.

pub mod commands;
pub mod credentials;
pub mod firmware_applications;
pub mod firmware_build;
pub mod firmware_distribution;
pub mod firmware_lifecycle;
pub mod firmware_publication;
pub mod firmware_release_source;
pub mod flasher;
pub mod linux_overview;
pub mod power;
pub mod skills_sync;
pub(crate) mod ssh;
pub mod ssh_terminal;
pub(crate) mod storage;
pub mod tunnel;
pub mod types;
pub mod workflow;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// Shared workbench state managed by Tauri.
pub struct WorkbenchState {
    pub db: Mutex<Connection>,
    pub cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub device_locks: Mutex<HashMap<i64, Arc<Mutex<()>>>>,
    pub flasher_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub tunnels: Mutex<HashMap<i64, tunnel::TunnelRuntime>>,
    pub ssh_terminals: Mutex<HashMap<String, ssh_terminal::SshTerminalRuntime>>,
}

impl WorkbenchState {
    /// Open the workbench database and apply idempotent migrations.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        storage::migrate(&connection).map_err(|e| e.to_string())?;
        firmware_lifecycle::migrate(&connection).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Mutex::new(connection),
            cancellations: Mutex::new(HashMap::new()),
            device_locks: Mutex::new(HashMap::new()),
            flasher_locks: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            ssh_terminals: Mutex::new(HashMap::new()),
        })
    }

    pub fn device_lock(&self, device_id: i64) -> Result<Arc<Mutex<()>>, String> {
        let mut locks = self.device_locks.lock().map_err(|e| e.to_string())?;
        Ok(locks
            .entry(device_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }

    pub fn flasher_lock(&self, kind: &str) -> Result<Arc<Mutex<()>>, String> {
        self.flasher_resource_lock(kind, "*")
    }

    /// Lock one explicitly selected flasher endpoint. Different physical USB
    /// endpoints may operate concurrently; an identical endpoint remains
    /// strictly exclusive even when reached through different callers.
    pub fn flasher_target_lock(
        &self,
        kind: &str,
        usb_binding: &str,
    ) -> Result<Arc<Mutex<()>>, String> {
        self.flasher_resource_lock(kind, usb_binding)
    }

    fn flasher_resource_lock(
        &self,
        kind: &str,
        usb_binding: &str,
    ) -> Result<Arc<Mutex<()>>, String> {
        let mut locks = self.flasher_locks.lock().map_err(|e| e.to_string())?;
        let key = format!("{kind}:{}", usb_binding.trim());
        Ok(locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WorkbenchState {
        WorkbenchState {
            db: Mutex::new(Connection::open_in_memory().unwrap()),
            cancellations: Mutex::new(HashMap::new()),
            device_locks: Mutex::new(HashMap::new()),
            flasher_locks: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            ssh_terminals: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn flasher_locks_are_scoped_to_the_bound_usb_endpoint() {
        let state = state();
        let first = state.flasher_target_lock("openixcli", "0:4").unwrap();
        let same = state.flasher_target_lock("openixcli", "0:4").unwrap();
        let second = state.flasher_target_lock("openixcli", "0:8").unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn fel_and_fes_reenumeration_share_one_physical_lock() {
        let state = state();
        let physical = "location:pciroot(0)#usbroot(0)#usb(4)";
        let fel = state.flasher_target_lock("openixcli", physical).unwrap();
        let fes = state.flasher_target_lock("openixcli", physical).unwrap();
        let other_board = state
            .flasher_target_lock("openixcli", "location:pciroot(0)#usbroot(0)#usb(5)")
            .unwrap();
        assert!(Arc::ptr_eq(&fel, &fes));
        assert!(!Arc::ptr_eq(&fel, &other_board));
    }
}

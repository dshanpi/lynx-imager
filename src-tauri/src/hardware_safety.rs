//! Process-wide arbitration for operations that can interrupt attached hardware.
//!
//! Linux/SDK and MCU keep separate business logic. This module is deliberately
//! domain-neutral: it only prevents two exclusive hardware operations from
//! overlapping and exposes enough state to refuse application shutdown.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareActivity {
    pub id: u64,
    pub owner: String,
    pub operation: String,
    pub scope: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareSafetyStatus {
    pub busy: bool,
    pub activities: Vec<String>,
}

pub struct HardwareSafetyState {
    activities: Arc<Mutex<HashMap<u64, ActiveHardwareActivity>>>,
    next_id: AtomicU64,
}

#[derive(Clone, Debug)]
struct ActiveHardwareActivity {
    activity: HardwareActivity,
    resource_key: Option<String>,
}

impl Default for HardwareSafetyState {
    fn default() -> Self {
        Self {
            activities: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        }
    }
}

#[derive(Debug)]
pub struct HardwareActivityGuard {
    id: u64,
    activities: Arc<Mutex<HashMap<u64, ActiveHardwareActivity>>>,
}

impl Drop for HardwareActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut current) = self.activities.lock() {
            current.remove(&self.id);
        }
    }
}

impl HardwareSafetyState {
    pub fn try_begin(
        &self,
        owner: &str,
        operation: &str,
        scope: impl Into<String>,
    ) -> Result<HardwareActivityGuard, String> {
        self.begin(owner, operation, scope.into(), None)
    }

    /// Start an operation that owns one explicitly bound hardware resource.
    ///
    /// Scoped operations may overlap only when their resource keys differ. A
    /// legacy/global operation remains exclusive with every scoped operation.
    pub fn try_begin_scoped(
        &self,
        owner: &str,
        operation: &str,
        scope: impl Into<String>,
        resource_key: impl Into<String>,
    ) -> Result<HardwareActivityGuard, String> {
        self.begin(owner, operation, scope.into(), Some(resource_key.into()))
    }

    fn begin(
        &self,
        owner: &str,
        operation: &str,
        scope: String,
        resource_key: Option<String>,
    ) -> Result<HardwareActivityGuard, String> {
        let mut activities = self.activities.lock().map_err(|error| error.to_string())?;
        let conflict = activities.values().any(|active| match &resource_key {
            None => true,
            Some(requested) => active
                .resource_key
                .as_ref()
                .is_none_or(|running| running == requested),
        });
        if conflict {
            return Err(format!(
                "HARDWARE_TASK_ACTIVE:已有冲突的硬件任务正在运行，已拒绝执行{operation}；请等待目标资源释放"
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let activity = HardwareActivity {
            id,
            owner: owner.to_owned(),
            operation: operation.to_owned(),
            scope,
        };
        activities.insert(
            id,
            ActiveHardwareActivity {
                activity,
                resource_key,
            },
        );
        Ok(HardwareActivityGuard {
            id,
            activities: self.activities.clone(),
        })
    }

    fn descriptions(&self) -> Vec<String> {
        self.activities
            .lock()
            .map(|activities| {
                activities
                    .values()
                    .map(|active| &active.activity)
                    .map(|item| {
                        if item.scope.is_empty() {
                            format!("{}：{}", item.owner, item.operation)
                        } else {
                            format!("{}：{}（{}）", item.owner, item.operation, item.scope)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub fn active_reasons<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(safety) = app.try_state::<HardwareSafetyState>() {
        reasons.extend(safety.descriptions());
    }
    if crate::flash::task::has_active_tasks() {
        reasons.push("通用烧录任务正在运行".to_owned());
    }
    if let Some(state) = app.try_state::<crate::flash::mass::MassProductionState>() {
        if state.0.lock().is_ok_and(|manager| manager.is_running()) {
            reasons.push("批量烧录任务正在运行".to_owned());
        }
    }
    if let Some(state) = app.try_state::<Arc<crate::workbench::WorkbenchState>>() {
        if let Ok(db) = state.db.lock() {
            let running = db
                .query_row(
                    "SELECT COUNT(*) FROM task_history WHERE status='running' AND kind IN ('flash','workflow','mainline_boot')",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap_or_default();
            if running > 0 {
                reasons.push(format!("Linux 工作台有 {running} 个硬件任务正在运行"));
            }
        }
    }
    if let Some(state) = app.try_state::<crate::mcu::McuState>() {
        if state.operation.try_lock().is_err() {
            reasons.push("MCU 硬件操作正在运行".to_owned());
        }
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

/// Return a conservative hardware-activity snapshot without waiting for any
/// business lock. Passive background work must defer when a lock is contended
/// instead of joining the hardware/task critical path.
pub fn activity_may_be_active_nonblocking<R: Runtime>(app: &AppHandle<R>) -> bool {
    if let Some(safety) = app.try_state::<HardwareSafetyState>() {
        match safety.activities.try_lock() {
            Ok(activities) if !activities.is_empty() => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    if crate::flash::task::has_active_tasks() {
        return true;
    }
    if let Some(state) = app.try_state::<crate::flash::mass::MassProductionState>() {
        match state.0.try_lock() {
            Ok(manager) if manager.is_running() => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    if let Some(state) = app.try_state::<Arc<crate::workbench::WorkbenchState>>() {
        let db = match state.db.try_lock() {
            Ok(db) => db,
            Err(_) => return true,
        };
        match db.query_row(
            "SELECT COUNT(*) FROM task_history WHERE status='running' AND kind IN ('flash','workflow','mainline_boot')",
            [],
            |row| row.get::<_, u64>(0),
        ) {
            Ok(running) if running > 0 => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    if let Some(state) = app.try_state::<crate::mcu::McuState>() {
        if state.operation.try_lock().is_err() {
            return true;
        }
    }
    false
}

pub fn ensure_idle<R: Runtime>(app: &AppHandle<R>, operation: &str) -> Result<(), String> {
    let reasons = active_reasons(app);
    if reasons.is_empty() {
        Ok(())
    } else {
        let error = format!(
            "HARDWARE_TASK_ACTIVE:{}；已拒绝执行{operation}",
            reasons.join("；")
        );
        log::debug!("{error}");
        Err(error)
    }
}

#[tauri::command]
pub fn hardware_safety_status(
    app: AppHandle,
    _state: State<'_, HardwareSafetyState>,
) -> HardwareSafetyStatus {
    let activities = active_reasons(&app);
    HardwareSafetyStatus {
        busy: !activities.is_empty(),
        activities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_activity_is_rejected_and_released_by_raii() {
        let state = HardwareSafetyState::default();
        let first = state.try_begin("Linux", "烧录", "board-1").unwrap();
        assert!(state
            .try_begin("MCU", "复位", "board-2")
            .unwrap_err()
            .starts_with("HARDWARE_TASK_ACTIVE:"));
        assert_eq!(state.descriptions(), ["Linux：烧录（board-1）"]);
        drop(first);
        assert!(state.try_begin("MCU", "复位", "board-2").is_ok());
    }

    #[test]
    fn different_scoped_resources_can_run_concurrently() {
        let state = HardwareSafetyState::default();
        let first = state
            .try_begin_scoped("Linux", "烧录", "board-1", "openixcli:0:4")
            .unwrap();
        let second = state
            .try_begin_scoped("Linux", "烧录", "board-2", "openixcli:0:8")
            .unwrap();
        assert_eq!(state.descriptions().len(), 2);
        assert!(state
            .try_begin_scoped("Linux", "主线 FEL 启动", "board-1", "openixcli:0:4")
            .is_err());
        assert!(state.try_begin("MCU", "全局操作", "probe").is_err());
        drop((first, second));
        assert!(state.descriptions().is_empty());
    }
}

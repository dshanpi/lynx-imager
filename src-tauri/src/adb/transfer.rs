use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use super::commands::AdbState;
use crate::{app_error::AppError, workbench::WorkbenchState};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AdbTransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdbTransferStartRequest {
    pub device_id: i64,
    pub direction: AdbTransferDirection,
    pub local_path: String,
    pub remote_path: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbTransferSnapshot {
    pub task_id: String,
    pub device_id: i64,
    pub direction: AdbTransferDirection,
    pub file_name: String,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
    pub status: String,
    pub error_code: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

struct TransferRuntime {
    snapshot: AdbTransferSnapshot,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct AdbTransferState {
    tasks: Mutex<HashMap<String, TransferRuntime>>,
}

impl AdbTransferState {
    fn update(
        &self,
        app: &AppHandle,
        task_id: &str,
        update: impl FnOnce(&mut AdbTransferSnapshot),
    ) {
        let snapshot = self.tasks.lock().ok().and_then(|mut tasks| {
            let runtime = tasks.get_mut(task_id)?;
            update(&mut runtime.snapshot);
            Some(runtime.snapshot.clone())
        });
        if let Some(snapshot) = snapshot {
            let _ = app.emit("adb-transfer-progress", snapshot);
        }
    }
}

#[tauri::command]
pub fn adb_transfer_start(
    app: AppHandle,
    adb: State<'_, AdbState>,
    transfers: State<'_, Arc<AdbTransferState>>,
    request: AdbTransferStartRequest,
) -> Result<AdbTransferSnapshot, AppError> {
    validate_request(&request).map_err(AppError::from)?;
    let (serial, device_lock) = {
        let workbench = app.state::<Arc<WorkbenchState>>();
        let db = workbench
            .db
            .lock()
            .map_err(|_| AppError::internal("WORKBENCH_DATABASE_LOCK_FAILED"))?;
        let device = crate::workbench::storage::device_by_id(&db, request.device_id)
            .map_err(|_| AppError::from("ADB_TRANSFER_DEVICE_NOT_FOUND"))?;
        if device.adb_serial.trim().is_empty() {
            return Err(AppError::from("ADB_DEVICE_NOT_BOUND_TO_LINUX_BOARD"));
        }
        let lock = workbench
            .device_lock(request.device_id)
            .map_err(AppError::from)?;
        (device.adb_serial, lock)
    };
    ensure_device_idle(transfers.inner(), request.device_id)?;
    let task_id = format!("adb-transfer-{:016x}", rand::random::<u64>());
    let file_name = transfer_file_name(&request);
    let started_at_ms = now_ms();
    let initial_total = if request.direction == AdbTransferDirection::Upload {
        std::fs::metadata(&request.local_path)
            .map_err(|error| AppError::from(format!("ADB_LOCAL_STAT:{error}")))?
            .len()
    } else {
        0
    };
    let snapshot = AdbTransferSnapshot {
        task_id: task_id.clone(),
        device_id: request.device_id,
        direction: request.direction,
        file_name,
        transferred_bytes: 0,
        total_bytes: initial_total,
        bytes_per_second: 0,
        status: "queued".to_owned(),
        error_code: None,
        started_at_ms,
        finished_at_ms: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let intent_id = publish_intent(&app, &snapshot).map_err(AppError::from)?;
    let safety_guard = app
        .state::<crate::hardware_safety::HardwareSafetyState>()
        .try_begin_scoped(
            "ADB",
            "文件传输",
            request.device_id.to_string(),
            format!("adb:{serial}"),
        )
        .map_err(AppError::from)?;
    let manager = adb
        .0
        .lock()
        .map_err(|_| AppError::internal("ADB_MANAGER_LOCK_FAILED"))?
        .clone();
    let device_was_claimed = {
        let mut tasks = transfers
            .tasks
            .lock()
            .map_err(|_| AppError::internal("ADB_TRANSFER_STATE_LOCK"))?;
        if tasks.values().any(|task| {
            task.snapshot.device_id == request.device_id
                && matches!(task.snapshot.status.as_str(), "queued" | "running")
        }) {
            true
        } else {
            tasks.insert(
                task_id.clone(),
                TransferRuntime {
                    snapshot: snapshot.clone(),
                    cancel: cancel.clone(),
                },
            );
            false
        }
    };
    if device_was_claimed {
        let error_code = "ADB_TRANSFER_DEVICE_BUSY".to_owned();
        let result: Result<(), String> = Err(error_code.clone());
        publish_result(
            &app,
            request.device_id,
            &task_id,
            &intent_id,
            &result,
            false,
        );
        return Err(AppError::from(error_code));
    }
    let worker_app = app.clone();
    let worker_state = transfers.inner().clone();
    let worker_task_id = task_id.clone();
    let worker_request = request.clone();
    let worker_cancel = cancel.clone();
    let worker_intent_id = intent_id.clone();
    let device_id = request.device_id;
    let spawn_result = std::thread::Builder::new()
        .name(format!("lynx-{task_id}"))
        .spawn(move || {
            let _safety_guard = safety_guard;
            let result = match device_lock.try_lock() {
                Ok(_device_guard) => run_transfer(
                    &worker_app,
                    &worker_state,
                    &worker_task_id,
                    &manager,
                    &serial,
                    &worker_request,
                    worker_cancel.clone(),
                ),
                Err(_) => Err("DEVICE_BUSY".to_owned()),
            };
            let cancelled = worker_cancel.load(Ordering::Acquire);
            worker_state.update(&worker_app, &worker_task_id, |snapshot| {
                snapshot.status = if cancelled {
                    "cancelled".to_owned()
                } else if result.is_ok() {
                    "success".to_owned()
                } else {
                    "failed".to_owned()
                };
                if result.is_ok() {
                    snapshot.transferred_bytes = snapshot.total_bytes;
                }
                snapshot.error_code = result.as_ref().err().map(|error| machine_error_code(error));
                snapshot.finished_at_ms = Some(now_ms());
            });
            publish_result(
                &worker_app,
                device_id,
                &worker_task_id,
                &worker_intent_id,
                &result,
                cancelled,
            );
            prune_finished(&worker_state, device_id);
        });
    if let Err(error) = spawn_result {
        if let Ok(mut tasks) = transfers.tasks.lock() {
            tasks.remove(&task_id);
        }
        let error_code = format!("ADB_TRANSFER_THREAD_START:{error}");
        let result: Result<(), String> = Err(error_code.clone());
        publish_result(
            &app,
            request.device_id,
            &task_id,
            &intent_id,
            &result,
            false,
        );
        return Err(AppError::from(error_code));
    }
    Ok(snapshot)
}

fn ensure_device_idle(state: &AdbTransferState, device_id: i64) -> Result<(), AppError> {
    let tasks = state
        .tasks
        .lock()
        .map_err(|_| AppError::internal("ADB_TRANSFER_STATE_LOCK"))?;
    if tasks.values().any(|task| {
        task.snapshot.device_id == device_id
            && matches!(task.snapshot.status.as_str(), "queued" | "running")
    }) {
        return Err(AppError::from("ADB_TRANSFER_DEVICE_BUSY"));
    }
    Ok(())
}

#[tauri::command]
pub fn adb_transfer_cancel(
    transfers: State<'_, Arc<AdbTransferState>>,
    task_id: String,
) -> Result<AdbTransferSnapshot, AppError> {
    let tasks = transfers
        .tasks
        .lock()
        .map_err(|_| AppError::internal("ADB_TRANSFER_STATE_LOCK"))?;
    let task = tasks
        .get(&task_id)
        .ok_or_else(|| AppError::from("ADB_TRANSFER_NOT_FOUND"))?;
    if matches!(task.snapshot.status.as_str(), "queued" | "running") {
        task.cancel.store(true, Ordering::Release);
    }
    Ok(task.snapshot.clone())
}

#[tauri::command]
pub fn adb_transfer_list(
    transfers: State<'_, Arc<AdbTransferState>>,
    device_id: i64,
) -> Result<Vec<AdbTransferSnapshot>, AppError> {
    let tasks = transfers
        .tasks
        .lock()
        .map_err(|_| AppError::internal("ADB_TRANSFER_STATE_LOCK"))?;
    let mut snapshots = tasks
        .values()
        .filter(|task| task.snapshot.device_id == device_id)
        .map(|task| task.snapshot.clone())
        .collect::<Vec<_>>();
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.started_at_ms));
    Ok(snapshots)
}

fn run_transfer(
    app: &AppHandle,
    state: &Arc<AdbTransferState>,
    task_id: &str,
    manager: &super::manager::AdbManager,
    serial: &str,
    request: &AdbTransferStartRequest,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    state.update(app, task_id, |snapshot| {
        snapshot.status = "running".to_owned()
    });
    let started = Instant::now();
    let last_emit = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
    let progress_state = state.clone();
    let progress_app = app.clone();
    let progress_task = task_id.to_owned();
    let progress = move |transferred: u64, total: u64| {
        let should_emit = last_emit
            .lock()
            .map(|mut last| {
                if last.elapsed() >= Duration::from_millis(100) || transferred >= total {
                    *last = Instant::now();
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if should_emit {
            progress_state.update(&progress_app, &progress_task, |snapshot| {
                snapshot.transferred_bytes = transferred;
                snapshot.total_bytes = total;
                snapshot.bytes_per_second = if started.elapsed().as_millis() == 0 {
                    0
                } else {
                    (u128::from(transferred) * 1_000 / started.elapsed().as_millis()) as u64
                };
            });
        }
    };
    match request.direction {
        AdbTransferDirection::Upload => {
            if !request.overwrite && manager.stat(Some(serial), &request.remote_path).is_ok() {
                return Err("ADB_REMOTE_EXISTS".to_owned());
            }
            let temporary = format!(
                "{}.lynx-part-{}",
                request.remote_path,
                short_task_id(task_id)
            );
            let result = manager.push_file_with_progress(
                serial,
                &request.local_path,
                &temporary,
                cancel.clone(),
                progress,
            );
            if result.is_ok() && !cancel.load(Ordering::Acquire) {
                manager
                    .rename(Some(serial), &temporary, &request.remote_path)
                    .map(|_| ())
            } else {
                let _ = manager.delete_file(Some(serial), &temporary);
                result
            }
        }
        AdbTransferDirection::Download => {
            let destination = PathBuf::from(&request.local_path);
            if destination.exists() && !request.overwrite {
                return Err("ADB_LOCAL_EXISTS".to_owned());
            }
            let temporary =
                destination.with_extension(format!("lynx-part-{}", short_task_id(task_id)));
            let temporary_text = temporary.to_string_lossy().into_owned();
            let result = manager.pull_file_with_progress(
                serial,
                &request.remote_path,
                &temporary_text,
                cancel.clone(),
                progress,
            );
            if result.is_ok() && !cancel.load(Ordering::Acquire) {
                commit_local_file(&temporary, &destination, request.overwrite, task_id)
            } else {
                let _ = std::fs::remove_file(&temporary);
                result
            }
        }
    }
}

fn commit_local_file(
    temporary: &Path,
    destination: &Path,
    overwrite: bool,
    task_id: &str,
) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(temporary, destination)
            .map_err(|error| format!("ADB_LOCAL_COMMIT:{error}"));
    }
    if !overwrite {
        return Err("ADB_LOCAL_EXISTS".to_owned());
    }
    let backup = destination.with_extension(format!("lynx-backup-{}", short_task_id(task_id)));
    std::fs::rename(destination, &backup).map_err(|error| format!("ADB_LOCAL_BACKUP:{error}"))?;
    match std::fs::rename(temporary, destination) {
        Ok(()) => {
            let _ = std::fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(backup, destination);
            Err(format!("ADB_LOCAL_COMMIT:{error}"))
        }
    }
}

fn validate_request(request: &AdbTransferStartRequest) -> Result<(), String> {
    if request.device_id <= 0 {
        return Err("ADB_TRANSFER_DEVICE_REQUIRED".to_owned());
    }
    for value in [&request.local_path, &request.remote_path] {
        if value.trim().is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
            return Err("ADB_TRANSFER_PATH_INVALID".to_owned());
        }
    }
    if !request.remote_path.starts_with('/') || request.remote_path.ends_with('/') {
        return Err("ADB_TRANSFER_REMOTE_FILE_REQUIRED".to_owned());
    }
    if request.direction == AdbTransferDirection::Upload
        && !Path::new(&request.local_path).is_file()
    {
        return Err("ADB_TRANSFER_LOCAL_FILE_REQUIRED".to_owned());
    }
    Ok(())
}

fn transfer_file_name(request: &AdbTransferStartRequest) -> String {
    let value = match request.direction {
        AdbTransferDirection::Upload => Path::new(&request.local_path)
            .file_name()
            .and_then(|value| value.to_str()),
        AdbTransferDirection::Download => request.remote_path.rsplit('/').next(),
    };
    value.unwrap_or("file").chars().take(180).collect()
}

fn short_task_id(task_id: &str) -> &str {
    task_id.rsplit('-').next().unwrap_or("transfer")
}

fn machine_error_code(error: &str) -> String {
    if error.contains("CANCELLED") || error.contains("Interrupted") {
        return "ADB_TRANSFER_CANCELLED".to_owned();
    }
    crate::audit::machine_error_code(error, "ADB_TRANSFER_FAILED")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn publish_intent(app: &AppHandle, snapshot: &AdbTransferSnapshot) -> Result<String, String> {
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "adb".to_owned(),
            action: "adb_file_transfer_intended".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", snapshot.device_id),
            session_id: Some(snapshot.task_id.clone()),
            session_kind: Some("adb_transfer".to_owned()),
            session_title: Some("ADB 文件传输".to_owned()),
            end_session: false,
            correlation_id: Some(snapshot.task_id.clone()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备执行当前单板的 ADB 文件传输".to_owned(),
            metadata: serde_json::json!({
                "direction": snapshot.direction,
                "fileNameByteCount": snapshot.file_name.len(),
                "declaredBytes": snapshot.total_bytes,
            }),
        },
    )?;
    match outcome {
        crate::audit::AuditWriteOutcome::Recorded { event_id } => Ok(event_id),
        crate::audit::AuditWriteOutcome::Degraded { .. } => {
            Err("AUDIT_REQUIRED_DEGRADED".to_owned())
        }
    }
}

fn publish_result(
    app: &AppHandle,
    device_id: i64,
    task_id: &str,
    causation_id: &str,
    result: &Result<(), String>,
    cancelled: bool,
) {
    let status = if cancelled {
        "cancelled"
    } else if result.is_ok() {
        "success"
    } else {
        "failed"
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "adb".to_owned(),
            action: "adb_file_transfer_completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{device_id}"),
            session_id: Some(task_id.to_owned()),
            session_kind: Some("adb_transfer".to_owned()),
            session_title: Some("ADB 文件传输".to_owned()),
            end_session: true,
            correlation_id: Some(task_id.to_owned()),
            causation_id: Some(causation_id.to_owned()),
            result: status.to_owned(),
            error_code: result.as_ref().err().map(|error| machine_error_code(error)),
            summary: format!("ADB 文件传输{status}"),
            metadata: serde_json::json!({}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn prune_finished(state: &AdbTransferState, device_id: i64) {
    let Ok(mut tasks) = state.tasks.lock() else {
        return;
    };
    let mut finished = tasks
        .iter()
        .filter(|(_, task)| {
            task.snapshot.device_id == device_id
                && !matches!(task.snapshot.status.as_str(), "queued" | "running")
        })
        .map(|(id, task)| (id.clone(), task.snapshot.started_at_ms))
        .collect::<Vec<_>>();
    finished.sort_by_key(|(_, started)| std::cmp::Reverse(*started));
    for (id, _) in finished.into_iter().skip(20) {
        tasks.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(task_id: &str, device_id: i64, status: &str) -> AdbTransferSnapshot {
        AdbTransferSnapshot {
            task_id: task_id.to_owned(),
            device_id,
            direction: AdbTransferDirection::Download,
            file_name: "file.bin".to_owned(),
            transferred_bytes: 0,
            total_bytes: 0,
            bytes_per_second: 0,
            status: status.to_owned(),
            error_code: None,
            started_at_ms: 1,
            finished_at_ms: None,
        }
    }

    #[test]
    fn transfer_requires_a_single_absolute_remote_file() {
        let request = AdbTransferStartRequest {
            device_id: 7,
            direction: AdbTransferDirection::Download,
            local_path: "/tmp/file.bin".to_owned(),
            remote_path: "/data/local/tmp/file.bin".to_owned(),
            overwrite: false,
        };
        assert!(validate_request(&request).is_ok());
        let mut invalid = request.clone();
        invalid.remote_path = "/data/local/tmp/".to_owned();
        assert_eq!(
            validate_request(&invalid).unwrap_err(),
            "ADB_TRANSFER_REMOTE_FILE_REQUIRED"
        );
    }

    #[test]
    fn one_board_cannot_start_two_active_transfers() {
        let state = AdbTransferState::default();
        state.tasks.lock().unwrap().insert(
            "first".to_owned(),
            TransferRuntime {
                snapshot: snapshot("first", 7, "running"),
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );

        assert_eq!(
            ensure_device_idle(&state, 7)
                .expect_err("active transfer must hold the board")
                .message,
            "ADB_TRANSFER_DEVICE_BUSY"
        );
        assert!(ensure_device_idle(&state, 8).is_ok());
        state
            .tasks
            .lock()
            .unwrap()
            .get_mut("first")
            .unwrap()
            .snapshot
            .status = "success".to_owned();
        assert!(ensure_device_idle(&state, 7).is_ok());
    }
}

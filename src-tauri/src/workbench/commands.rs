use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tauri::{command, AppHandle, Emitter, Manager, State};

use super::ssh;
use super::storage;
use super::types::{
    platform_catalog, platform_spec, AiChannelTestResult, AiProvider, CameraDeviceCandidate,
    DeviceConfig, HostConfig, PlatformSpec, ProjectConfig, SshCredentials, SshTestResult,
    TaskEvent, TaskHistory, WorkflowStepHistory,
};
use super::WorkbenchState;

#[derive(Clone, Copy, Debug)]
pub(crate) enum RemoteTaskActor {
    User,
    Gateway,
}

pub(crate) fn camera_binding_key(camera: &nokhwa::utils::CameraInfo) -> String {
    let mut fingerprint = Sha256::new();
    fingerprint.update(camera.index().as_string().as_bytes());
    fingerprint.update([0]);
    fingerprint.update(camera.human_name().as_bytes());
    fingerprint.update([0]);
    fingerprint.update(camera.description().as_bytes());
    fingerprint.update([0]);
    fingerprint.update(camera.misc().as_bytes());
    let digest = hex::encode(fingerprint.finalize());
    format!("camera-{}", &digest[..32])
}

impl RemoteTaskActor {
    fn identity(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user", "local-user"),
            Self::Gateway => ("ai", "linux-mcp-gateway"),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteAuditContext {
    task_id: String,
    session_id: String,
    operation: &'static str,
    actor: RemoteTaskActor,
    scope_type: &'static str,
    scope_id: String,
    host_id: i64,
    project_id: Option<i64>,
    intent_id: String,
}

pub(crate) fn begin_remote_task_audit(
    app: &AppHandle,
    task_id: &str,
    operation: &'static str,
    actor: RemoteTaskActor,
    host: &HostConfig,
    project: Option<&ProjectConfig>,
    safe_metadata: serde_json::Value,
) -> Result<RemoteAuditContext, String> {
    let host_id = host.id.ok_or_else(|| "SSH_HOST_ID_REQUIRED".to_owned())?;
    let project_id = project.and_then(|project| project.id);
    let (scope_type, scope_id) = match project.and_then(|project| project.device_id) {
        Some(device_id) => ("linux_board", format!("linux:{device_id}")),
        None => match project_id {
            Some(project_id) => ("workbench_project", format!("project:{project_id}")),
            None => ("ssh_host", format!("ssh-host:{host_id}")),
        },
    };
    let (actor_type, actor_id) = actor.identity();
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "ssh".to_owned(),
            action: format!("ssh_{operation}_intended"),
            scope_type: scope_type.to_owned(),
            scope_id: scope_id.clone(),
            session_id: Some(format!("{task_id}:ssh:{operation}")),
            session_kind: Some("ssh_task".to_owned()),
            session_title: Some("远程主机任务".to_owned()),
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: format!("准备执行受控 SSH {operation} 任务"),
            metadata: safe_metadata,
        },
    )?;
    let crate::audit::AuditWriteOutcome::Recorded { event_id } = outcome else {
        return Err("AUDIT_REQUIRED_DEGRADED".to_owned());
    };
    Ok(RemoteAuditContext {
        task_id: task_id.to_owned(),
        session_id: format!("{task_id}:ssh:{operation}"),
        operation,
        actor,
        scope_type,
        scope_id,
        host_id,
        project_id,
        intent_id: event_id,
    })
}

fn publish_ssh_connected(app: &AppHandle, context: &RemoteAuditContext) {
    publish_remote_audit(
        app,
        context,
        "ssh_connected",
        "success",
        None,
        "SSH 已建立可信连接",
        false,
    );
}

pub(crate) fn finish_remote_task_audit<T>(
    app: &AppHandle,
    context: &RemoteAuditContext,
    status: &str,
    result: &Result<T, String>,
) {
    let error_code = remote_audit_error_code(status, result);
    publish_remote_audit(
        app,
        context,
        &format!("ssh_{}_completed", context.operation),
        status,
        error_code,
        if status == "success" {
            "SSH 远程任务完成"
        } else {
            "SSH 远程任务结束"
        },
        true,
    );
}

fn remote_audit_error_code<T>(status: &str, result: &Result<T, String>) -> Option<String> {
    match (status, result) {
        ("success", _) => None,
        ("cancelled", _) => Some("TASK_CANCELLED".to_owned()),
        ("failed", Ok(_)) => Some("SSH_REMOTE_EXIT_NONZERO".to_owned()),
        (_, Err(error)) => Some(crate::audit::machine_error_code(error, "SSH_TASK_FAILED")),
        _ => Some("SSH_TASK_FAILED".to_owned()),
    }
}

fn publish_remote_audit(
    app: &AppHandle,
    context: &RemoteAuditContext,
    action: &str,
    result: &str,
    error_code: Option<String>,
    summary: &str,
    end_session: bool,
) {
    let (actor_type, actor_id) = context.actor.identity();
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "ssh".to_owned(),
            action: action.to_owned(),
            scope_type: context.scope_type.to_owned(),
            scope_id: context.scope_id.clone(),
            session_id: Some(context.session_id.clone()),
            session_kind: Some("ssh_task".to_owned()),
            session_title: Some("远程主机任务".to_owned()),
            end_session,
            correlation_id: Some(context.task_id.clone()),
            causation_id: Some(context.intent_id.clone()),
            result: result.to_owned(),
            error_code,
            summary: summary.to_owned(),
            metadata: serde_json::json!({
                "hostId": context.host_id,
                "projectId": context.project_id,
            }),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

struct ConnectionTestAudit {
    session_id: String,
    intent_id: String,
    scope_id: String,
    test_type: &'static str,
    started: std::time::Instant,
}

fn begin_connection_test(
    app: &AppHandle,
    host: &HostConfig,
    test_type: &'static str,
    allow_untrusted: bool,
) -> Result<ConnectionTestAudit, String> {
    let session_id = format!(
        "connection-test-{}-{:016x}",
        now_ms(),
        rand::random::<u64>()
    );
    let scope_id = host.id.map_or_else(
        || format!("ssh-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("ssh-host:{id}"),
    );
    let outcome = crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "connection".to_owned(),
            action: format!("{test_type}_test_intended"),
            scope_type: "ssh_host".to_owned(),
            scope_id: scope_id.clone(),
            session_id: Some(session_id.clone()),
            session_kind: Some("connection_test".to_owned()),
            session_title: Some("远程连接测试".to_owned()),
            end_session: false,
            correlation_id: Some(session_id.clone()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备测试远程连接".to_owned(),
            metadata: serde_json::json!({"hostId": host.id, "testType": test_type, "authType": host.auth_type.as_str(), "allowUntrusted": allow_untrusted}),
        },
    )?;
    let crate::audit::AuditWriteOutcome::Recorded { event_id } = outcome else {
        return Err("AUDIT_REQUIRED_DEGRADED".to_owned());
    };
    Ok(ConnectionTestAudit {
        session_id,
        intent_id: event_id,
        scope_id,
        test_type,
        started: std::time::Instant::now(),
    })
}

fn finish_connection_test<T>(
    app: &AppHandle,
    audit: &ConnectionTestAudit,
    result: &Result<T, String>,
) {
    let (status, error_code) = match result {
        Ok(_) => ("success", None),
        Err(error) => (
            "failed",
            Some(crate::audit::machine_error_code(
                error,
                "CONNECTION_TEST_FAILED",
            )),
        ),
    };
    let _ = crate::audit::publish(
        app,
        crate::audit::AuditEventDraft {
            actor_type: "user".to_owned(),
            actor_id: "local-user".to_owned(),
            category: "connection".to_owned(),
            action: format!("{}_test_completed", audit.test_type),
            scope_type: "ssh_host".to_owned(),
            scope_id: audit.scope_id.clone(),
            session_id: Some(audit.session_id.clone()),
            session_kind: Some("connection_test".to_owned()),
            session_title: Some("远程连接测试".to_owned()),
            end_session: true,
            correlation_id: Some(audit.session_id.clone()),
            causation_id: Some(audit.intent_id.clone()),
            result: status.to_owned(),
            error_code,
            summary: "远程连接测试结束".to_owned(),
            metadata: serde_json::json!({"testType": audit.test_type, "latencyMs": audit.started.elapsed().as_millis()}),
        },
        crate::audit::AuditRequirement::BestEffort,
    );
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[command]
pub fn workbench_list_hosts(
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<HostConfig>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    storage::list_hosts(&db).map_err(|e| e.to_string())
}

#[command]
pub fn workbench_list_tasks(
    limit: Option<u32>,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<TaskHistory>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    storage::list_tasks(&db, limit.unwrap_or(100)).map_err(|e| e.to_string())
}

#[command]
pub fn workbench_list_workflow_steps(
    task_id: String,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<WorkflowStepHistory>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    storage::list_workflow_steps(&db, &task_id).map_err(|e| e.to_string())
}

#[command]
pub fn workbench_save_host(
    app: AppHandle,
    mut host: HostConfig,
    connection_test_session_id: Option<String>,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<i64, String> {
    validate_host(&host)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    if host.id.is_none() {
        if let Some(existing) = storage::list_hosts(&db)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|existing| same_ssh_identity(existing, &host))
        {
            // SDK channels share one persisted host (and therefore one keyring
            // credential) when they connect as the same SSH account. Preserve
            // the trusted host configuration instead of replacing it with the
            // intentionally blank fields from the new-channel form.
            let mut reused = existing;
            if !host.host_fingerprint.trim().is_empty() {
                reused.host_fingerprint = host.host_fingerprint;
            }
            host = reused;
        }
    }
    let old = host.id.and_then(|id| storage::host_by_id(&db, id).ok());
    let fields = crate::config_audit::changed_fields(
        old.as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            .as_ref(),
        serde_json::to_value(&host).ok().as_ref(),
        &[
            "name",
            "address",
            "port",
            "username",
            "authType",
            "privateKeyPath",
            "hostFingerprint",
            "defaultWorkDir",
            "aiProvider",
            "aiExecutable",
            "aiArgsJson",
        ],
    );
    let operation = if host.id.is_some() {
        "update"
    } else {
        "create"
    };
    let scope_id = host.id.map_or_else(
        || format!("ssh-host-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("ssh-host:{id}"),
    );
    drop(db);
    let audit = crate::config_audit::begin_linked(
        &app,
        "ssh_host",
        scope_id,
        "ssh_host",
        operation,
        fields,
        serde_json::json!({"authType": host.auth_type.as_str(), "aiProvider": host.ai_provider.as_str(), "trusted": !host.host_fingerprint.is_empty(), "connectionTestLinked": connection_test_session_id.is_some()}),
        connection_test_session_id,
    )?;
    let result = (|| {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::save_host(&db, &host).map_err(|e| e.to_string())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[command]
pub fn workbench_list_projects(
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<ProjectConfig>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    storage::list_projects(&db).map_err(|e| e.to_string())
}

#[command]
pub fn workbench_save_project(
    app: AppHandle,
    project: ProjectConfig,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<i64, String> {
    validate_project_config(&project)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let existing = storage::list_projects(&db).map_err(|e| e.to_string())?;
    validate_project_conflicts(&project, &existing)?;
    let old = project
        .id
        .and_then(|id| storage::project_by_id(&db, id).ok());
    let fields = crate::config_audit::changed_fields(
        old.as_ref()
            .and_then(|v| serde_json::to_value(v).ok())
            .as_ref(),
        serde_json::to_value(&project).ok().as_ref(),
        &[
            "name",
            "hostId",
            "deviceId",
            "sdkDir",
            "buildCommand",
            "firmwarePath",
            "localDownloadDir",
            "verifySha256",
            "localGatewayPort",
            "remoteForwardPort",
            "mcpServerName",
            "aiProvider",
        ],
    );
    let operation = if project.id.is_some() {
        "update"
    } else {
        "create"
    };
    let scope_id = project.id.map_or_else(
        || format!("project-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("project:{id}"),
    );
    drop(db);
    let audit = crate::config_audit::begin(
        &app,
        "workbench_project",
        scope_id,
        "workbench_project",
        operation,
        fields,
        serde_json::json!({"hostId": project.host_id, "deviceBound": project.device_id.is_some(), "verify": project.verify_sha256, "aiProvider": project.ai_provider.as_str()}),
    )?;
    let result = (|| {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let existing = storage::list_projects(&db).map_err(|e| e.to_string())?;
        validate_project_conflicts(&project, &existing)?;
        storage::save_project(&db, &project).map_err(|e| e.to_string())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[command]
pub fn workbench_list_devices(
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<DeviceConfig>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    storage::list_devices(&db).map_err(|e| e.to_string())
}

#[command]
pub fn imager_resolve_flash_target(
    state: State<'_, Arc<WorkbenchState>>,
    name: String,
    chip_vendor: String,
    flasher_kind: String,
    flasher_path: String,
    usb_binding: String,
) -> Result<DeviceConfig, String> {
    validate_device_vendor(&name, &chip_vendor, &flasher_kind)?;
    if usb_binding.trim().is_empty() {
        return Err("请选择 USB 端口。".to_owned());
    }
    let db = state.db.lock().map_err(|e| e.to_string())?;
    if !flasher_path.is_empty()
        && !storage::list_devices(&db)
            .map_err(|e| e.to_string())?
            .iter()
            .any(|d| {
                d.chip_vendor == chip_vendor
                    && d.flasher_kind == flasher_kind
                    && d.flasher_path == flasher_path
            })
    {
        return Err("烧录工具配置已变化，请重新检测。".to_owned());
    }
    storage::basic_flash_target(
        &db,
        &name,
        &chip_vendor,
        &flasher_kind,
        &flasher_path,
        &usb_binding,
    )
    .map_err(|e| e.to_string())
}

#[command]
pub fn workbench_list_platforms() -> Vec<PlatformSpec> {
    platform_catalog().to_vec()
}

#[command]
pub async fn workbench_list_cameras() -> Result<Vec<CameraDeviceCandidate>, String> {
    let query = tauri::async_runtime::spawn_blocking(|| {
        let backend =
            nokhwa::native_api_backend().ok_or_else(|| "CAMERA_BACKEND_UNAVAILABLE".to_owned())?;
        std::panic::catch_unwind(|| nokhwa::query(backend))
            .map_err(|_| "CAMERA_ENUMERATION_FAILED".to_owned())?
            .map_err(|error| format!("CAMERA_ENUMERATION_FAILED:{error}"))
    });
    let cameras = tokio::time::timeout(Duration::from_secs(3), query)
        .await
        .map_err(|_| "CAMERA_ENUMERATION_TIMEOUT".to_owned())?
        .map_err(|error| format!("CAMERA_ENUMERATION_FAILED:{error}"))??;
    Ok(cameras
        .into_iter()
        .map(|camera| CameraDeviceCandidate {
            key: camera_binding_key(&camera),
            name: camera.human_name(),
            description: camera.description().to_owned(),
        })
        .collect())
}

#[command]
pub fn workbench_save_device(
    app: AppHandle,
    device: DeviceConfig,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<i64, String> {
    validate_device_config(&device)?;
    let device_lock = device
        .id
        .map(|device_id| state.device_lock(device_id))
        .transpose()?;
    let _device_guard = device_lock
        .as_ref()
        .map(|lock| {
            lock.try_lock().map_err(|_| {
                "DEVICE_BUSY:board configuration cannot change during a hardware operation"
                    .to_owned()
            })
        })
        .transpose()?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let existing = storage::list_devices(&db).map_err(|e| e.to_string())?;
    validate_device_conflicts(&device, &existing)?;
    let old = device.id.and_then(|id| storage::device_by_id(&db, id).ok());
    let fields = crate::config_audit::changed_fields(
        old.as_ref()
            .and_then(|v| serde_json::to_value(v).ok())
            .as_ref(),
        serde_json::to_value(&device).ok().as_ref(),
        &[
            "name",
            "boardImagePath",
            "chipVendor",
            "serialPort",
            "serialVid",
            "serialPid",
            "serialNumber",
            "usbBinding",
            "cameraBinding",
            "adbSerial",
            "serialInterfaceNumber",
            "baudRate",
            "serialConsoleType",
            "serialLoginMode",
            "serialLoginUsername",
            "serialLoginPrompt",
            "serialPasswordPrompt",
            "serialShellPrompt",
            "serialAutoLogin",
            "flasherKind",
            "flasherPath",
            "flasherArgs",
            "powerController",
            "powerControllerVid",
            "powerControllerPid",
            "powerControllerSerial",
            "powerControllerInterfaceNumber",
            "powerBaudRate",
            "powerProfileJson",
            "powerChannel",
            "bootChannel",
            "resetChannel",
            "powerOffWaitMs",
            "powerOnWaitMs",
            "serialReconnectTimeoutMs",
            "bootTimeoutMs",
        ],
    );
    let operation = if device.id.is_some() {
        "update"
    } else {
        "create"
    };
    let scope_id = device.id.map_or_else(
        || format!("linux-candidate:{:016x}", rand::random::<u64>()),
        |id| format!("linux:{id}"),
    );
    drop(db);
    let audit = crate::config_audit::begin(
        &app,
        "linux_board",
        scope_id,
        "linux_board",
        operation,
        fields,
        serde_json::json!({"chipVendor": device.chip_vendor, "flasherKind": device.flasher_kind, "serialBound": !device.serial_port.is_empty(), "adbBound": !device.adb_serial.is_empty(), "powerBound": !device.power_controller.is_empty()}),
    )?;
    let result = (|| {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let existing = storage::list_devices(&db).map_err(|e| e.to_string())?;
        validate_device_conflicts(&device, &existing)?;
        storage::save_device(&db, &device).map_err(|e| e.to_string())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

fn validate_device_config(device: &DeviceConfig) -> Result<(), String> {
    validate_device_vendor(&device.name, &device.chip_vendor, &device.flasher_kind)?;
    validate_serial_login_profile(device)?;
    super::power::validate_configuration_profile(device)
}

fn validate_serial_login_profile(device: &DeviceConfig) -> Result<(), String> {
    if !matches!(
        device.serial_console_type.as_str(),
        "auto" | "linux" | "rt_smart"
    ) {
        return Err("SERIAL_CONSOLE_TYPE_INVALID".to_owned());
    }
    if !matches!(
        device.serial_login_mode.as_str(),
        "none" | "username" | "password"
    ) {
        return Err("SERIAL_LOGIN_MODE_INVALID".to_owned());
    }
    for (name, value) in [
        ("username", device.serial_login_username.as_str()),
        ("loginPrompt", device.serial_login_prompt.as_str()),
        ("passwordPrompt", device.serial_password_prompt.as_str()),
        ("shellPrompt", device.serial_shell_prompt.as_str()),
    ] {
        if value.is_empty()
            || value.chars().count() > 64
            || value.chars().any(|character| character.is_control())
        {
            return Err(format!("SERIAL_LOGIN_FIELD_INVALID:{name}"));
        }
    }
    if device.serial_auto_login
        && (device.serial_console_type == "rt_smart" || device.serial_login_mode == "none")
    {
        return Err("SERIAL_AUTO_LOGIN_CONFIGURATION_INVALID".to_owned());
    }
    Ok(())
}

fn validate_device_conflicts(
    device: &DeviceConfig,
    existing: &[DeviceConfig],
) -> Result<(), String> {
    for other in existing.iter().filter(|other| other.id != device.id) {
        if other.name.trim().eq_ignore_ascii_case(device.name.trim()) {
            return Err(format!(
                "DEVICE_NAME_CONFLICT:board name {} is already in use",
                other.name
            ));
        }
        let same_serial_identity = if !device.serial_number.trim().is_empty() {
            let same_usb_identity = device.serial_number == other.serial_number
                && (device.serial_vid == 0
                    || other.serial_vid == 0
                    || device.serial_vid == other.serial_vid)
                && (device.serial_pid == 0
                    || other.serial_pid == 0
                    || device.serial_pid == other.serial_pid);
            let distinct_named_ports = !device.serial_port.trim().is_empty()
                && !other.serial_port.trim().is_empty()
                && !device.serial_port.eq_ignore_ascii_case(&other.serial_port);
            let same_usb_interface = match (
                device.serial_interface_number,
                other.serial_interface_number,
            ) {
                (Some(left), Some(right)) => left == right,
                _ => {
                    device.serial_port.trim().is_empty()
                        || other.serial_port.trim().is_empty()
                        || device.serial_port.eq_ignore_ascii_case(&other.serial_port)
                }
            };
            same_usb_identity && !distinct_named_ports && same_usb_interface
        } else {
            !device.serial_port.trim().is_empty() && device.serial_port == other.serial_port
        };
        if same_serial_identity {
            return Err(format!(
                "DEVICE_BINDING_CONFLICT:serial is already assigned to board {}",
                other.name
            ));
        }
        if !device.usb_binding.trim().is_empty() && device.usb_binding == other.usb_binding {
            return Err(format!(
                "DEVICE_BINDING_CONFLICT:flash USB {} is already assigned to board {}",
                device.usb_binding, other.name
            ));
        }
        if !device.adb_serial.trim().is_empty() && device.adb_serial == other.adb_serial {
            return Err(format!(
                "DEVICE_BINDING_CONFLICT:ADB {} is already assigned to board {}",
                device.adb_serial, other.name
            ));
        }
        if let (Some((controller, channels)), Some((other_controller, other_channels))) = (
            super::power::ch347_channel_claim(device)?,
            super::power::ch347_channel_claim(other)?,
        ) {
            if controller == other_controller {
                if let Some(channel) = channels
                    .iter()
                    .find(|channel| other_channels.contains(channel))
                {
                    return Err(format!(
                        "POWER_CHANNEL_ASSIGNED:GPIO{channel} on {controller} is already assigned to board {}",
                        other.name
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_device_vendor(name: &str, chip_vendor: &str, flasher_kind: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("VALIDATION:device name is required".to_owned());
    }
    let platform = platform_spec(chip_vendor)
        .ok_or_else(|| format!("CHIP_VENDOR_UNSUPPORTED:{chip_vendor}"))?;
    if flasher_kind != platform.flasher {
        return Err(format!(
            "FLASHER_VENDOR_MISMATCH:{chip_vendor} requires {}",
            platform.flasher
        ));
    }
    if let Some(detected_vendor) = vendor_hint_from_name(name) {
        if detected_vendor != chip_vendor {
            return Err(format!(
                "CHIP_VENDOR_IDENTITY_MISMATCH:board name identifies {detected_vendor}, but configuration selects {chip_vendor}"
            ));
        }
    }
    Ok(())
}

fn vendor_hint_from_name(name: &str) -> Option<&'static str> {
    let normalized = name.trim().to_lowercase();
    let compact: String = normalized
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    if normalized.contains("rockchip")
        || normalized.contains("瑞芯微")
        || compact
            .strip_prefix("rk")
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|character| character.is_ascii_digit())
    {
        return Some("rockchip");
    }
    if normalized.contains("allwinner")
        || normalized.contains("sunxi")
        || normalized.contains("全志")
        || ["t113", "t153", "d1", "h3", "h5", "h6", "h616", "v853"]
            .iter()
            .any(|prefix| compact.starts_with(prefix))
    {
        return Some("allwinner");
    }
    if normalized.contains("nxp") || compact.contains("imx") {
        return Some("nxp");
    }
    if normalized.contains("kendryte")
        || normalized.contains("canaan")
        || normalized.contains("嘉楠")
        || compact.contains("k230")
    {
        return Some("kendryte");
    }
    if compact.contains("stm32mp") || normalized.contains("意法半导体") {
        return Some("st");
    }
    None
}

#[command]
pub fn workbench_delete_config(
    app: AppHandle,
    kind: String,
    id: i64,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<(), String> {
    if id <= 0 || !matches!(kind.as_str(), "host" | "project" | "device") {
        return Err("CONFIG_RESOURCE_INVALID".to_owned());
    }
    let (scope_type, scope_id) = match kind.as_str() {
        "host" => ("ssh_host", format!("ssh-host:{id}")),
        "project" => ("workbench_project", format!("project:{id}")),
        _ => ("linux_board", format!("linux:{id}")),
    };
    let audit = crate::config_audit::begin(
        &app,
        scope_type,
        scope_id,
        "workbench_config",
        "delete",
        vec![],
        serde_json::json!({"kind": kind}),
    )?;
    let result = (|| {
        if kind == "device" {
            // A locked/unavailable desktop keyring must not make an already
            // confirmed board deletion lie about the database result. Board
            // IDs are AUTOINCREMENT and are never reused, so an unavailable
            // orphan entry cannot attach to another board.
            let _ = super::credentials::delete_serial_login_password(id);
        }
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::delete_config(&db, &kind, id).map_err(|e| e.to_string())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

/// Test authentication and return the observed SHA-256 host fingerprint.
///
/// The fingerprint is only persisted after explicit confirmation by the UI.
#[command]
pub async fn workbench_test_ssh(
    app: AppHandle,
    host: HostConfig,
    credentials: SshCredentials,
    allow_untrusted: bool,
) -> Result<SshTestResult, String> {
    validate_host(&host)?;
    // Keep the IPC field for compatibility, but never authenticate without a pin.
    if allow_untrusted {
        return Err("SSH_TRUST_CONFIRMATION_REQUIRED:请核验主机指纹后再测试连接".to_owned());
    }
    let audit = begin_connection_test(&app, &host, "ssh", false)?;
    let credentials = match super::credentials::resolve(host.id, credentials) {
        Ok(credentials) => credentials,
        Err(error) => {
            let result: Result<SshTestResult, String> = Err(error);
            finish_connection_test(&app, &audit, &result);
            return result;
        }
    };
    let audit_session_id = audit.session_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_, fingerprint, trusted) = ssh::connect(&host, &credentials)?;
        Ok(SshTestResult {
            fingerprint,
            trusted,
            audit_session_id,
        })
    })
    .await
    .map_err(|_| "SSH_TEST_TASK_FAILED".to_owned())?;
    finish_connection_test(&app, &audit, &result);
    result
}

/// Verify that the selected coding-agent CLI is available on the Ubuntu host.
#[command]
pub async fn workbench_test_ai_channel(
    app: AppHandle,
    host: HostConfig,
    credentials: SshCredentials,
) -> Result<AiChannelTestResult, String> {
    validate_host(&host)?;
    validate_ai_config(&host)?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }
    let audit = begin_connection_test(&app, &host, "ai_channel", false)?;
    let credentials = match super::credentials::resolve(host.id, credentials) {
        Ok(credentials) => credentials,
        Err(error) => {
            let result: Result<AiChannelTestResult, String> = Err(error);
            finish_connection_test(&app, &audit, &result);
            return result;
        }
    };
    let audit_session_id = audit.session_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let executable = resolved_ai_executable(&host)?;
        let (session, _, _) = ssh::connect(&host, &credentials)?;
        let mut channel = session
            .channel_session()
            .map_err(|e| format!("SSH_CHANNEL:{e}"))?;
        let inner = format!("exec {} --version", ssh::shell_quote(&executable));
        let command = format!("bash -lc {}", ssh::shell_quote(&inner));
        channel
            .exec(&command)
            .map_err(|e| format!("AI_CHANNEL_EXEC:{e}"))?;
        let mut stdout = String::new();
        let mut stderr = String::new();
        channel
            .read_to_string(&mut stdout)
            .map_err(|e| format!("AI_CHANNEL_STDOUT:{e}"))?;
        channel
            .stderr()
            .read_to_string(&mut stderr)
            .map_err(|e| format!("AI_CHANNEL_STDERR:{e}"))?;
        channel
            .wait_close()
            .map_err(|e| format!("AI_CHANNEL_CLOSE:{e}"))?;
        let status = channel
            .exit_status()
            .map_err(|e| format!("AI_CHANNEL_EXIT:{e}"))?;
        if status != 0 {
            return Err("AI_CHANNEL_UNAVAILABLE".to_owned());
        }
        let version = if stdout.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        Ok(AiChannelTestResult {
            provider: host.ai_provider.as_str().to_owned(),
            executable,
            version: version.chars().take(4096).collect(),
            audit_session_id,
        })
    })
    .await
    .map_err(|_| "AI_CHANNEL_TEST_TASK_FAILED".to_owned())?;
    finish_connection_test(&app, &audit, &result);
    result
}

/// Run one non-interactive coding-agent task on the project's Ubuntu host.
#[command]
pub fn workbench_run_ai_task(
    project_id: i64,
    prompt: String,
    credentials: SshCredentials,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    let (host, project) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let project = storage::project_by_id(&db, project_id).map_err(|e| e.to_string())?;
        let host = storage::host_by_id(&db, project.host_id).map_err(|e| e.to_string())?;
        (host, project)
    };
    validate_executable_project(&project)?;
    validate_ai_config(&host)?;
    if prompt.trim().is_empty() || prompt.len() > 128 * 1024 || prompt.contains('\0') {
        return Err("VALIDATION:AI prompt must be 1-131072 bytes and contain no NUL".to_owned());
    }
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }
    let credentials = super::credentials::resolve(host.id, credentials)?;
    let task_id = new_task_id("ai");
    let audit = begin_remote_task_audit(
        &app,
        &task_id,
        "ai_task",
        RemoteTaskActor::User,
        &host,
        Some(&project),
        serde_json::json!({
            "promptByteCount": prompt.len(),
            "provider": host.ai_provider.as_str(),
        }),
    )?;
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    insert_task(&state, &task_id, "ai", Some(project_id))?;

    let state = state.inner().clone();
    let return_id = task_id.clone();
    std::thread::spawn(move || {
        let result = run_ai_task(
            &app,
            &task_id,
            &host,
            &project,
            &prompt,
            &credentials,
            &cancellation,
            &audit,
        );
        let (status, exit_code, error) = match &result {
            Ok(code) if *code == 0 => ("success", Some(*code), String::new()),
            Ok(code) => (
                "failed",
                Some(*code),
                format!("remote AI command exited with {code}"),
            ),
            Err(error) if cancellation.load(Ordering::Relaxed) => {
                ("cancelled", None, error.clone())
            }
            Err(error) => ("failed", None, error.clone()),
        };
        finish_task(&state, &task_id, status, exit_code, &error);
        finish_remote_task_audit(&app, &audit, status, &result);
        emit_status(&app, &task_id, "ai", status, &error);
        if let Ok(mut tasks) = state.cancellations.lock() {
            tasks.remove(&task_id);
        }
    });
    Ok(return_id)
}

/// Run the project's pre-configured command and stream stdout/stderr as events.
#[command]
pub fn workbench_run_build(
    project_id: i64,
    credentials: SshCredentials,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    let (host, project) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let project = storage::project_by_id(&db, project_id).map_err(|e| e.to_string())?;
        let host = storage::host_by_id(&db, project.host_id).map_err(|e| e.to_string())?;
        (host, project)
    };
    let credentials = super::credentials::resolve(host.id, credentials)?;
    validate_executable_project(&project)?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }

    let task_id = new_task_id("build");
    let audit = begin_remote_task_audit(
        &app,
        &task_id,
        "build",
        RemoteTaskActor::User,
        &host,
        Some(&project),
        serde_json::json!({"buildCommandByteCount": project.build_command.len()}),
    )?;
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    insert_task(&state, &task_id, "build", Some(project_id))?;

    let state = state.inner().clone();
    let return_id = task_id.clone();
    std::thread::spawn(move || {
        let result = run_build_task(
            &app,
            &task_id,
            &host,
            &project,
            &credentials,
            &cancellation,
            &audit,
        );
        let (status, exit_code, error) = match &result {
            Ok(code) if *code == 0 => ("success", Some(*code), String::new()),
            Ok(code) => (
                "failed",
                Some(*code),
                format!("remote command exited with {code}"),
            ),
            Err(error) if cancellation.load(Ordering::Relaxed) => {
                ("cancelled", None, error.clone())
            }
            Err(error) => ("failed", None, error.clone()),
        };
        finish_task(&state, &task_id, status, exit_code, &error);
        finish_remote_task_audit(&app, &audit, status, &result);
        emit(
            &app,
            TaskEvent {
                task_id: task_id.clone(),
                phase: "build".to_owned(),
                stream: "system".to_owned(),
                message: error,
                current: None,
                total: None,
                status: status.to_owned(),
            },
        );
        if let Ok(mut tasks) = state.cancellations.lock() {
            tasks.remove(&task_id);
        }
    });
    Ok(return_id)
}

/// Download the project's configured artifact through SFTP.
#[command]
pub fn workbench_download_firmware(
    project_id: i64,
    credentials: SshCredentials,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    start_download_firmware(
        project_id,
        None,
        None,
        credentials,
        app,
        state.inner(),
        RemoteTaskActor::User,
    )
}

/// Start an SFTP firmware download using a saved SDK channel.
///
/// Kept separate from the Tauri command wrapper so the desktop-hosted MCP
/// gateway can reuse the exact same task, credential, hashing, and event path.
pub(crate) fn start_download_firmware(
    project_id: i64,
    remote_path: Option<String>,
    local_download_dir: Option<String>,
    credentials: SshCredentials,
    app: AppHandle,
    state: &Arc<WorkbenchState>,
    actor: RemoteTaskActor,
) -> Result<String, String> {
    let (host, mut project) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let project = storage::project_by_id(&db, project_id).map_err(|e| e.to_string())?;
        let host = storage::host_by_id(&db, project.host_id).map_err(|e| e.to_string())?;
        (host, project)
    };
    if let Some(remote_path) = remote_path {
        project.firmware_path = remote_path;
    }
    if let Some(local_download_dir) = local_download_dir {
        project.local_download_dir = local_download_dir;
        project.verify_sha256 = true;
    }
    let credentials = super::credentials::resolve(host.id, credentials)?;
    validate_download_project(&project)?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }

    let task_id = new_task_id("transfer");
    let audit = begin_remote_task_audit(
        &app,
        &task_id,
        "download",
        actor,
        &host,
        Some(&project),
        serde_json::json!({
            "remotePathByteCount": project.firmware_path.len(),
            "verifySha256": project.verify_sha256,
        }),
    )?;
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    insert_task(state, &task_id, "transfer", Some(project_id))?;

    let state = state.clone();
    let return_id = task_id.clone();
    std::thread::spawn(move || {
        let result = download_task(
            &app,
            &task_id,
            &host,
            &project,
            &credentials,
            &cancellation,
            &audit,
        );
        let (status, error) = match &result {
            Ok(_) => ("success", String::new()),
            Err(error) if cancellation.load(Ordering::Relaxed) => ("cancelled", error.clone()),
            Err(error) => ("failed", error.clone()),
        };
        finish_task(&state, &task_id, status, None, &error);
        finish_remote_task_audit(&app, &audit, status, &result);
        emit(
            &app,
            TaskEvent {
                task_id: task_id.clone(),
                phase: "transfer".to_owned(),
                stream: "system".to_owned(),
                message: error,
                current: None,
                total: None,
                status: status.to_owned(),
            },
        );
        if let Ok(mut tasks) = state.cancellations.lock() {
            tasks.remove(&task_id);
        }
    });
    Ok(return_id)
}

/// Upload a local file to Ubuntu using a remote `.part` file and atomic rename.
#[command]
#[allow(clippy::too_many_arguments)]
pub fn workbench_upload_file(
    host_id: i64,
    local_path: String,
    remote_path: String,
    overwrite: bool,
    credentials: SshCredentials,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    let host = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::host_by_id(&db, host_id).map_err(|e| e.to_string())?
    };
    let credentials = super::credentials::resolve(host.id, credentials)?;
    if host.host_fingerprint.is_empty() {
        return Err("HOST_KEY_CONFIRM_REQUIRED:test and trust this host first".to_owned());
    }
    let local = PathBuf::from(local_path);
    if !local.is_file() || !remote_path.starts_with('/') {
        return Err("VALIDATION:local file and absolute remote path are required".to_owned());
    }
    let task_id = new_task_id("upload");
    let audit = begin_remote_task_audit(
        &app,
        &task_id,
        "upload",
        RemoteTaskActor::User,
        &host,
        None,
        serde_json::json!({
            "localPathByteCount": local.as_os_str().len(),
            "remotePathByteCount": remote_path.len(),
            "overwrite": overwrite,
        }),
    )?;
    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancellation.clone());
    insert_task(&state, &task_id, "upload", None)?;
    let workbench = state.inner().clone();
    let returned = task_id.clone();
    std::thread::spawn(move || {
        let result = upload_task(
            &app,
            &task_id,
            &host,
            &credentials,
            &local,
            &remote_path,
            overwrite,
            &cancellation,
            &audit,
        );
        let (status, error) = match &result {
            Ok(()) => ("success", String::new()),
            Err(error) if cancellation.load(Ordering::Relaxed) => ("cancelled", error.clone()),
            Err(error) => ("failed", error.clone()),
        };
        finish_task(&workbench, &task_id, status, None, &error);
        finish_remote_task_audit(&app, &audit, status, &result);
        emit_status(&app, &task_id, "upload", status, &error);
        if let Ok(mut tasks) = workbench.cancellations.lock() {
            tasks.remove(&task_id);
        }
    });
    Ok(returned)
}

#[allow(clippy::too_many_arguments)]
fn upload_task(
    app: &AppHandle,
    task_id: &str,
    host: &HostConfig,
    credentials: &SshCredentials,
    local_path: &std::path::Path,
    remote_path: &str,
    overwrite: bool,
    cancellation: &AtomicBool,
    audit: &RemoteAuditContext,
) -> Result<(), String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_sftp_upload");
    let total = local_path
        .metadata()
        .map_err(|e| format!("LOCAL_STAT:{e}"))?
        .len();
    let mut local = std::fs::File::open(local_path).map_err(|e| format!("LOCAL_OPEN:{e}"))?;
    let (session, _, _) = ssh::connect(host, credentials)?;
    publish_ssh_connected(app, audit);
    let sftp = session.sftp().map_err(|e| format!("SFTP_OPEN:{e}"))?;
    let target = std::path::Path::new(remote_path);
    if !overwrite && sftp.stat(target).is_ok() {
        return Err(format!("REMOTE_EXISTS:{remote_path}"));
    }
    let partial_path = format!("{remote_path}.part");
    let partial = std::path::Path::new(&partial_path);
    let mut remote = sftp
        .create(partial)
        .map_err(|e| format!("SFTP_REMOTE_CREATE:{e}"))?;
    let mut transferred = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancellation.load(Ordering::Relaxed) {
            return Err(format!(
                "Upload cancelled; partial file kept at {partial_path}"
            ));
        }
        let count = local
            .read(&mut buffer)
            .map_err(|e| format!("LOCAL_READ:{e}"))?;
        if count == 0 {
            break;
        }
        remote
            .write_all(&buffer[..count])
            .map_err(|e| format!("SFTP_WRITE:{e}"))?;
        transferred += count as u64;
        emit(
            app,
            TaskEvent {
                task_id: task_id.to_owned(),
                phase: "upload".to_owned(),
                stream: "progress".to_owned(),
                message: remote_path.to_owned(),
                current: Some(transferred),
                total: Some(total),
                status: "running".to_owned(),
            },
        );
    }
    remote.flush().map_err(|e| format!("SFTP_FLUSH:{e}"))?;
    drop(remote);
    let flags = if overwrite {
        Some(ssh2::RenameFlags::OVERWRITE | ssh2::RenameFlags::ATOMIC)
    } else {
        Some(ssh2::RenameFlags::ATOMIC)
    };
    sftp.rename(partial, target, flags)
        .map_err(|e| format!("SFTP_RENAME:{e}"))
}

#[command]
pub fn workbench_cancel_task(
    task_id: String,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<(), String> {
    let tasks = state.cancellations.lock().map_err(|e| e.to_string())?;
    let cancellation = tasks
        .get(&task_id)
        .ok_or_else(|| "TASK_NOT_RUNNING".to_owned())?;
    cancellation.store(true, Ordering::Relaxed);
    Ok(())
}

pub(crate) fn run_build_task(
    app: &AppHandle,
    task_id: &str,
    host: &HostConfig,
    project: &ProjectConfig,
    credentials: &SshCredentials,
    cancellation: &AtomicBool,
    audit: &RemoteAuditContext,
) -> Result<i32, String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_build");
    emit_status(app, task_id, "build", "running", "Connecting to build host");
    let (session, _, _) = ssh::connect(host, credentials)?;
    publish_ssh_connected(app, audit);
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("SSH_CHANNEL:{e}"))?;
    let command = format!(
        "python3 -c {} {} {}",
        ssh::shell_quote(include_str!("project_lock.py")),
        ssh::shell_quote(&project.sdk_dir),
        ssh::shell_quote(&project.build_command)
    );
    channel
        .exec(&command)
        .map_err(|e| format!("SSH_EXEC:{e}"))?;
    session.set_blocking(false);
    let mut stdout = [0u8; 8192];
    let mut stderr = [0u8; 8192];

    loop {
        if cancellation.load(Ordering::Relaxed) {
            let _ = channel.close();
            return Err("Task cancelled by user".to_owned());
        }
        let mut received = false;
        match channel.read(&mut stdout) {
            Ok(count) if count > 0 => {
                received = true;
                emit_output(app, task_id, "build", "stdout", &stdout[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("SSH_STDOUT:{error}")),
            _ => {}
        }
        match channel.stderr().read(&mut stderr) {
            Ok(count) if count > 0 => {
                received = true;
                emit_output(app, task_id, "build", "stderr", &stderr[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("SSH_STDERR:{error}")),
            _ => {}
        }
        if channel.eof() {
            break;
        }
        if !received {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    session.set_blocking(true);
    channel.wait_close().map_err(|e| format!("SSH_CLOSE:{e}"))?;
    channel.exit_status().map_err(|e| format!("SSH_EXIT:{e}"))
}

#[allow(clippy::too_many_arguments)]
fn run_ai_task(
    app: &AppHandle,
    task_id: &str,
    host: &HostConfig,
    project: &ProjectConfig,
    prompt: &str,
    credentials: &SshCredentials,
    cancellation: &AtomicBool,
    audit: &RemoteAuditContext,
) -> Result<i32, String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_ai_channel");
    let provider = host.ai_provider.as_str();
    emit_status(
        app,
        task_id,
        "ai",
        "running",
        &format!("Connecting to {provider} on Ubuntu"),
    );
    let (session, _, _) = ssh::connect(host, credentials)?;
    publish_ssh_connected(app, audit);
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("SSH_CHANNEL:{e}"))?;
    let command = build_ai_command(host, &project.sdk_dir, prompt)?;
    channel
        .exec(&command)
        .map_err(|e| format!("AI_CHANNEL_EXEC:{e}"))?;
    session.set_blocking(false);
    let mut claude_stream =
        matches!(host.ai_provider, AiProvider::ClaudeCode).then(ClaudeStreamDecoder::default);
    let mut stdout = [0u8; 8192];
    let mut stderr = [0u8; 8192];
    loop {
        if cancellation.load(Ordering::Relaxed) {
            let _ = channel.close();
            return Err("AI task cancelled by user".to_owned());
        }
        let mut received = false;
        match channel.read(&mut stdout) {
            Ok(count) if count > 0 => {
                received = true;
                if let Some(decoder) = claude_stream.as_mut() {
                    for message in decoder.push(&stdout[..count]) {
                        emit_output(app, task_id, "ai", "stdout", message.as_bytes());
                    }
                } else {
                    emit_output(app, task_id, "ai", "stdout", &stdout[..count]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("AI_CHANNEL_STDOUT:{error}")),
            _ => {}
        }
        match channel.stderr().read(&mut stderr) {
            Ok(count) if count > 0 => {
                received = true;
                emit_output(app, task_id, "ai", "stderr", &stderr[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("AI_CHANNEL_STDERR:{error}")),
            _ => {}
        }
        if channel.eof() {
            break;
        }
        if !received {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    if let Some(decoder) = claude_stream.as_mut() {
        for message in decoder.finish() {
            emit_output(app, task_id, "ai", "stdout", message.as_bytes());
        }
    }
    session.set_blocking(true);
    channel
        .wait_close()
        .map_err(|e| format!("AI_CHANNEL_CLOSE:{e}"))?;
    channel
        .exit_status()
        .map_err(|e| format!("AI_CHANNEL_EXIT:{e}"))
}

pub(crate) fn download_task(
    app: &AppHandle,
    task_id: &str,
    host: &HostConfig,
    project: &ProjectConfig,
    credentials: &SshCredentials,
    cancellation: &AtomicBool,
    audit: &RemoteAuditContext,
) -> Result<PathBuf, String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_sftp_download");
    emit_status(app, task_id, "transfer", "running", "Opening SFTP transfer");
    let (session, _, _) = ssh::connect(host, credentials)?;
    publish_ssh_connected(app, audit);
    let remote_path = resolve_remote_artifact(&session, &project.firmware_path)?;
    let expected_sha256 = if project.verify_sha256 {
        Some(remote_sha256(&session, &remote_path)?)
    } else {
        None
    };
    let sftp = session.sftp().map_err(|e| format!("SFTP_OPEN:{e}"))?;
    let mut remote = sftp
        .open(std::path::Path::new(&remote_path))
        .map_err(|e| format!("SFTP_REMOTE_OPEN:{e}"))?;
    let total = remote
        .stat()
        .map_err(|e| format!("SFTP_STAT:{e}"))?
        .size
        .unwrap_or(0);
    let filename = remote_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "SFTP_PATH:firmware path has no file name".to_owned())?;
    let target_dir = PathBuf::from(&project.local_download_dir);
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("LOCAL_DIR:{e}"))?;
    let target = target_dir.join(filename);
    let mut replace_target = false;
    if target.exists() {
        if let Some(expected) = &expected_sha256 {
            let actual = local_sha256(&target)?;
            if expected.eq_ignore_ascii_case(&actual) {
                persist_transfer_progress(app, task_id, total, total, &target, Some(expected));
                emit_status(
                    app,
                    task_id,
                    "transfer",
                    "success",
                    &format!("Using verified cached firmware {}", target.display()),
                );
                return Ok(target);
            }
            replace_target = true;
        } else {
            return Err(format!("LOCAL_EXISTS:{}", target.display()));
        }
    }
    let partial = target_dir.join(format!("{filename}.part"));
    let mut local = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&partial)
        .map_err(|e| format!("LOCAL_OPEN:{e}"))?;
    let mut hasher = Sha256::new();
    let mut transferred = local
        .metadata()
        .map_err(|e| format!("LOCAL_STAT:{e}"))?
        .len();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut last_persisted_percent = None;
    if total > 0 && transferred > total {
        local.set_len(0).map_err(|e| format!("LOCAL_RESET:{e}"))?;
        transferred = 0;
    }
    if transferred > 0 {
        local
            .seek(SeekFrom::Start(0))
            .map_err(|e| format!("LOCAL_SEEK:{e}"))?;
        let mut remaining = transferred;
        while remaining > 0 {
            let limit = usize::try_from(remaining)
                .unwrap_or(buffer.len())
                .min(buffer.len());
            let count = local
                .read(&mut buffer[..limit])
                .map_err(|e| format!("LOCAL_RESUME_READ:{e}"))?;
            if count == 0 {
                return Err("LOCAL_RESUME_READ:partial file ended unexpectedly".to_owned());
            }
            hasher.update(&buffer[..count]);
            remaining -= count as u64;
        }
        remote
            .seek(SeekFrom::Start(transferred))
            .map_err(|e| format!("SFTP_RESUME_SEEK:{e}"))?;
        local
            .seek(SeekFrom::Start(transferred))
            .map_err(|e| format!("LOCAL_RESUME_SEEK:{e}"))?;
        emit_status(
            app,
            task_id,
            "transfer",
            "running",
            &format!("Resuming transfer at {transferred}/{total} bytes"),
        );
    } else {
        local.set_len(0).map_err(|e| format!("LOCAL_RESET:{e}"))?;
    }
    loop {
        if cancellation.load(Ordering::Relaxed) {
            return Err(format!(
                "Transfer cancelled; partial file kept at {}",
                partial.display()
            ));
        }
        let count = remote
            .read(&mut buffer)
            .map_err(|e| format!("SFTP_READ:{e}"))?;
        if count == 0 {
            break;
        }
        local
            .write_all(&buffer[..count])
            .map_err(|e| format!("LOCAL_WRITE:{e}"))?;
        hasher.update(&buffer[..count]);
        transferred += count as u64;
        let percent = if total == 0 {
            None
        } else {
            Some(((transferred as f64 / total as f64) * 100.0).min(100.0))
        };
        let whole_percent = percent.map(|value| value.floor() as u32);
        if whole_percent != last_persisted_percent {
            persist_transfer_progress(app, task_id, transferred, total, &target, None);
            last_persisted_percent = whole_percent;
        }
        emit(
            app,
            TaskEvent {
                task_id: task_id.to_owned(),
                phase: "transfer".to_owned(),
                stream: "progress".to_owned(),
                message: filename.to_owned(),
                current: Some(transferred),
                total: Some(total),
                status: "running".to_owned(),
            },
        );
    }
    local.sync_all().map_err(|e| format!("LOCAL_SYNC:{e}"))?;
    if total > 0 && transferred != total {
        return Err(format!("SIZE_MISMATCH:remote={total},local={transferred}"));
    }
    if let Some(expected) = expected_sha256.as_deref() {
        let actual = format!("{:x}", hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            local.set_len(0).map_err(|e| format!("LOCAL_RESET:{e}"))?;
            return Err(format!("SHA256_MISMATCH:remote={expected},local={actual}"));
        }
    }
    if replace_target {
        std::fs::remove_file(&target).map_err(|e| format!("LOCAL_CACHE_REPLACE:{e}"))?;
    }
    std::fs::rename(&partial, &target).map_err(|e| format!("LOCAL_RENAME:{e}"))?;
    persist_transfer_progress(
        app,
        task_id,
        transferred,
        total,
        &target,
        expected_sha256.as_deref(),
    );
    Ok(target)
}

fn persist_transfer_progress(
    app: &AppHandle,
    task_id: &str,
    transferred: u64,
    total: u64,
    local_path: &Path,
    sha256: Option<&str>,
) {
    let progress = (total > 0).then(|| ((transferred as f64 / total as f64) * 100.0).min(100.0));
    let details = serde_json::json!({
        "bytesTransferred": transferred,
        "totalBytes": total,
        "localPath": local_path,
        "sha256": sha256,
    });
    let state = app.state::<Arc<WorkbenchState>>();
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "UPDATE task_history SET progress=?1,details_json=?2 WHERE task_id=?3",
            rusqlite::params![progress, details.to_string(), task_id],
        );
    };
}

fn local_sha256(path: &Path) -> Result<String, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| format!("LOCAL_CACHE_OPEN:{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("LOCAL_CACHE_READ:{error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn resolve_remote_artifact(
    session: &ssh2::Session,
    configured: &str,
) -> Result<String, String> {
    if !configured.contains(['*', '?', '[']) {
        return Ok(configured.to_owned());
    }
    let wildcard = configured
        .find(['*', '?', '['])
        .ok_or_else(|| "FIRMWARE_PATTERN_INVALID".to_owned())?;
    let prefix = &configured[..wildcard];
    let root = prefix
        .rfind('/')
        .map(|index| if index == 0 { "/" } else { &prefix[..index] })
        .unwrap_or(".");
    let command = format!(
        "find -- {} -type f -path {} -printf '%T@\\t%p\\n' 2>/dev/null | sort -nr | head -n 1",
        ssh::shell_quote(root),
        ssh::shell_quote(configured)
    );
    session.set_blocking(true);
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("SSH_CHANNEL:{e}"))?;
    channel
        .exec(&command)
        .map_err(|e| format!("FIRMWARE_SEARCH:{e}"))?;
    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|e| format!("FIRMWARE_SEARCH:{e}"))?;
    channel
        .wait_close()
        .map_err(|e| format!("FIRMWARE_SEARCH:{e}"))?;
    if channel
        .exit_status()
        .map_err(|e| format!("FIRMWARE_SEARCH:{e}"))?
        != 0
    {
        return Err(format!(
            "FIRMWARE_SEARCH_FAILED:root={root},pattern={configured}"
        ));
    }
    output
        .trim()
        .split_once('\t')
        .map(|(_, path)| path.to_owned())
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("FIRMWARE_NOT_FOUND:root={root},pattern={configured}"))
}

fn remote_sha256(session: &ssh2::Session, path: &str) -> Result<String, String> {
    session.set_blocking(true);
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("SSH_CHANNEL:{e}"))?;
    channel
        .exec(&format!("sha256sum -- {}", ssh::shell_quote(path)))
        .map_err(|e| format!("SSH_SHA256:{e}"))?;
    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|e| format!("SSH_SHA256:{e}"))?;
    channel
        .wait_close()
        .map_err(|e| format!("SSH_SHA256:{e}"))?;
    let status = channel
        .exit_status()
        .map_err(|e| format!("SSH_SHA256:{e}"))?;
    if status != 0 {
        return Err(format!("SSH_SHA256:remote command exited with {status}"));
    }
    output
        .split_whitespace()
        .next()
        .filter(|hash| hash.len() == 64)
        .map(str::to_owned)
        .ok_or_else(|| "SSH_SHA256:invalid remote output".to_owned())
}

fn validate_host(host: &HostConfig) -> Result<(), String> {
    if host.name.trim().is_empty()
        || host.address.trim().is_empty()
        || host.username.trim().is_empty()
    {
        return Err("VALIDATION:host name, address and username are required".to_owned());
    }
    if host.port == 0 {
        return Err("VALIDATION:SSH port must be greater than zero".to_owned());
    }
    Ok(())
}

fn same_ssh_identity(left: &HostConfig, right: &HostConfig) -> bool {
    left.address
        .trim()
        .eq_ignore_ascii_case(right.address.trim())
        && left.port == right.port
        && left.username.trim() == right.username.trim()
}

fn validate_ai_config(host: &HostConfig) -> Result<(), String> {
    let executable = resolved_ai_executable(host)?;
    if executable.contains(['\0', '\n', '\r']) {
        return Err("VALIDATION:AI executable contains invalid characters".to_owned());
    }
    let _ = resolved_ai_args(host)?;
    Ok(())
}

fn resolved_ai_executable(host: &HostConfig) -> Result<String, String> {
    let executable = if host.ai_executable.trim().is_empty() {
        host.ai_provider.default_executable()
    } else {
        host.ai_executable.trim()
    };
    if executable.is_empty() {
        return Err("VALIDATION:custom AI executable is required".to_owned());
    }
    Ok(executable.to_owned())
}

fn resolved_ai_args(host: &HostConfig) -> Result<Vec<String>, String> {
    if !host.ai_args_json.trim().is_empty() {
        let args = serde_json::from_str::<Vec<String>>(&host.ai_args_json)
            .map_err(|e| format!("VALIDATION:AI arguments must be a JSON string array: {e}"))?;
        if args.iter().any(|arg| arg.contains('\0')) {
            return Err("VALIDATION:AI arguments cannot contain NUL".to_owned());
        }
        return Ok(args);
    }
    Ok(match host.ai_provider {
        AiProvider::Codex => vec![
            "exec".to_owned(),
            "--sandbox".to_owned(),
            "workspace-write".to_owned(),
            "--skip-git-repo-check".to_owned(),
        ],
        AiProvider::ClaudeCode => vec![
            "--print".to_owned(),
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--verbose".to_owned(),
            "--include-partial-messages".to_owned(),
            "--permission-mode".to_owned(),
            "acceptEdits".to_owned(),
        ],
        AiProvider::DeepseekHarness => Vec::new(),
        AiProvider::Custom => Vec::new(),
    })
}

fn build_ai_command(host: &HostConfig, work_dir: &str, prompt: &str) -> Result<String, String> {
    let executable = resolved_ai_executable(host)?;
    let args = resolved_ai_args(host)?;
    let mut parts = Vec::with_capacity(args.len() + 2);
    parts.push(ssh::shell_quote(&executable));
    parts.extend(args.iter().map(|arg| ssh::shell_quote(arg)));
    parts.push(ssh::shell_quote(prompt));
    let inner = format!(
        "cd -- {} && exec {}",
        ssh::shell_quote(work_dir),
        parts.join(" ")
    );
    Ok(format!("bash -lc {}", ssh::shell_quote(&inner)))
}

#[derive(Default)]
struct ClaudeStreamDecoder {
    pending: Vec<u8>,
    emitted_text: bool,
}

impl ClaudeStreamDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut messages = Vec::new();
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=position).collect::<Vec<_>>();
            if let Some(message) = self.decode_line(&line[..line.len().saturating_sub(1)]) {
                messages.push(message);
            }
        }
        messages
    }

    fn finish(&mut self) -> Vec<String> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let line = std::mem::take(&mut self.pending);
        self.decode_line(&line).into_iter().collect()
    }

    fn decode_line(&mut self, line: &[u8]) -> Option<String> {
        if line.is_empty() {
            return None;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            return Some(format!("{}\n", String::from_utf8_lossy(line)));
        };
        if let Some(text) = value
            .get("event")
            .and_then(|event| event.get("delta"))
            .and_then(|delta| delta.get("text"))
            .and_then(serde_json::Value::as_str)
        {
            self.emitted_text = true;
            return Some(text.to_owned());
        }
        if value.get("type").and_then(serde_json::Value::as_str) == Some("result")
            && !self.emitted_text
        {
            return value
                .get("result")
                .and_then(serde_json::Value::as_str)
                .map(|result| format!("{result}\n"));
        }
        None
    }
}

fn validate_project_config(project: &ProjectConfig) -> Result<(), String> {
    if project.name.trim().is_empty()
        || project.mcp_server_name.trim().is_empty()
        || project.local_gateway_port == 0
        || project.remote_forward_port == 0
    {
        return Err("VALIDATION:通道名称、MCP 名称和端口不能为空".to_owned());
    }
    if project.name.contains('\0')
        || project.sdk_dir.contains('\0')
        || project.mcp_server_name.contains('\0')
    {
        return Err("VALIDATION:NUL bytes are not allowed".to_owned());
    }
    if !project
        .mcp_server_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err("VALIDATION:MCP 名称只能包含 ASCII 字母、数字、下划线和连字符".to_owned());
    }
    Ok(())
}

fn validate_project_conflicts(
    project: &ProjectConfig,
    existing: &[ProjectConfig],
) -> Result<(), String> {
    for other in existing.iter().filter(|other| other.id != project.id) {
        if other.local_gateway_port == project.local_gateway_port {
            return Err(format!(
                "CHANNEL_CONFLICT:本机 Gateway 端口 {} 已被 SDK 通道“{}”占用",
                project.local_gateway_port, other.name
            ));
        }
        if other.host_id == project.host_id
            && other.remote_forward_port == project.remote_forward_port
        {
            return Err(format!(
                "CHANNEL_CONFLICT:远端 MCP 端口 {} 已被 SDK 通道“{}”占用",
                project.remote_forward_port, other.name
            ));
        }
        if other.host_id == project.host_id && other.mcp_server_name == project.mcp_server_name {
            return Err(format!(
                "CHANNEL_CONFLICT:MCP 名称“{}”已被 SDK 通道“{}”占用",
                project.mcp_server_name, other.name
            ));
        }
    }
    Ok(())
}

fn validate_executable_project(project: &ProjectConfig) -> Result<(), String> {
    validate_project_config(project)?;
    if project.name.trim().is_empty()
        || !project.sdk_dir.starts_with('/')
        || project.build_command.trim().is_empty()
        || !project.firmware_path.starts_with('/')
        || project.local_download_dir.trim().is_empty()
    {
        return Err(
            "VALIDATION:project name, absolute SDK/artifact paths, build command and local directory are required"
                .to_owned(),
        );
    }
    if project.build_command.contains('\0') || project.sdk_dir.contains('\0') {
        return Err("VALIDATION:NUL bytes are not allowed".to_owned());
    }
    Ok(())
}

fn validate_download_project(project: &ProjectConfig) -> Result<(), String> {
    validate_project_config(project)?;
    if !project.firmware_path.starts_with('/') || project.local_download_dir.trim().is_empty() {
        return Err(
            "VALIDATION:absolute remote artifact path and local download directory are required"
                .to_owned(),
        );
    }
    if project.firmware_path.contains('\0') || project.local_download_dir.contains('\0') {
        return Err("VALIDATION:NUL bytes are not allowed".to_owned());
    }
    Ok(())
}

pub(crate) fn insert_task(
    state: &WorkbenchState,
    task_id: &str,
    kind: &str,
    project_id: Option<i64>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO task_history(task_id,kind,project_id,status,started_at,phase)\
         VALUES(?1,?2,?3,'running',?4,?2)",
        rusqlite::params![task_id, kind, project_id, unix_seconds()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn finish_task(
    state: &WorkbenchState,
    task_id: &str,
    status: &str,
    exit_code: Option<i32>,
    error: &str,
) {
    if let Ok(db) = state.db.lock() {
        let _ = storage::transition_task_status(&db, task_id, status, exit_code, error);
    }
}

fn emit_output(app: &AppHandle, task_id: &str, phase: &str, stream: &str, bytes: &[u8]) {
    emit(
        app,
        TaskEvent {
            task_id: task_id.to_owned(),
            phase: phase.to_owned(),
            stream: stream.to_owned(),
            message: String::from_utf8_lossy(bytes).into_owned(),
            current: None,
            total: None,
            status: "running".to_owned(),
        },
    );
}

pub(crate) fn emit_status(
    app: &AppHandle,
    task_id: &str,
    phase: &str,
    status: &str,
    message: &str,
) {
    emit(
        app,
        TaskEvent {
            task_id: task_id.to_owned(),
            phase: phase.to_owned(),
            stream: "system".to_owned(),
            message: message.to_owned(),
            current: None,
            total: None,
            status: status.to_owned(),
        },
    );
}

pub(crate) fn emit(app: &AppHandle, event: TaskEvent) {
    let _ = app.emit("workbench-task-event", event);
}

pub(crate) fn new_task_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::types::SshAuthType;
    use nokhwa::utils::{CameraIndex, CameraInfo};

    #[test]
    fn camera_binding_fingerprint_rejects_a_different_device_at_the_same_index() {
        let first = CameraInfo::new(
            "UVC Camera",
            "/dev/video0",
            "usb-1-2",
            CameraIndex::Index(0),
        );
        let replacement = CameraInfo::new(
            "UVC Camera",
            "/dev/video0",
            "usb-1-3",
            CameraIndex::Index(0),
        );

        let first_key = camera_binding_key(&first);
        assert!(first_key.starts_with("camera-"));
        assert_eq!(first_key, camera_binding_key(&first));
        assert_ne!(first_key, camera_binding_key(&replacement));
    }

    fn host(provider: AiProvider) -> HostConfig {
        HostConfig {
            id: None,
            name: "ubuntu".to_owned(),
            address: "127.0.0.1".to_owned(),
            port: 22,
            username: "developer".to_owned(),
            auth_type: SshAuthType::PrivateKey,
            private_key_path: "/tmp/key".to_owned(),
            host_fingerprint: "fingerprint".to_owned(),
            default_work_dir: "/srv".to_owned(),
            ai_provider: provider,
            ai_executable: String::new(),
            ai_args_json: String::new(),
        }
    }

    #[test]
    fn remote_audit_result_distinguishes_exit_failure_and_cancellation() {
        let failed_exit: Result<i32, String> = Ok(7);
        let cancelled: Result<i32, String> = Err("Task cancelled by user".to_owned());
        let transport: Result<i32, String> = Err("SSH_CONNECT:refused".to_owned());
        assert_eq!(
            remote_audit_error_code("failed", &failed_exit),
            Some("SSH_REMOTE_EXIT_NONZERO".to_owned())
        );
        assert_eq!(
            remote_audit_error_code("cancelled", &cancelled),
            Some("TASK_CANCELLED".to_owned())
        );
        assert_eq!(
            remote_audit_error_code("failed", &transport),
            Some("SSH_CONNECT".to_owned())
        );
    }

    fn board(id: i64, name: &str) -> DeviceConfig {
        DeviceConfig {
            id: Some(id),
            board_uid: format!("linux-{id:032x}"),
            name: name.to_owned(),
            board_image_path: String::new(),
            chip_vendor: "rockchip".to_owned(),
            serial_port: format!("/dev/ttyACM{id}"),
            serial_vid: 0x1234,
            serial_pid: 0x5678,
            serial_number: format!("SERIAL-{id}"),
            usb_binding: format!("1:{id}"),
            camera_binding: String::new(),
            adb_serial: format!("ADB-{id}"),
            serial_interface_number: Some(1),
            baud_rate: 115_200,
            serial_console_type: "auto".to_owned(),
            serial_login_mode: "none".to_owned(),
            serial_login_username: "root".to_owned(),
            serial_login_prompt: "login:".to_owned(),
            serial_password_prompt: "Password:".to_owned(),
            serial_shell_prompt: "#".to_owned(),
            serial_auto_login: false,
            flasher_kind: "openrockcli".to_owned(),
            flasher_path: String::new(),
            flasher_args: String::new(),
            power_controller: String::new(),
            power_controller_vid: 0,
            power_controller_pid: 0,
            power_controller_serial: String::new(),
            power_controller_interface_number: None,
            power_baud_rate: 115_200,
            power_profile_json: String::new(),
            power_channel: String::new(),
            boot_channel: String::new(),
            reset_channel: String::new(),
            power_off_wait_ms: 1_000,
            power_on_wait_ms: 2_000,
            serial_reconnect_timeout_ms: 30_000,
            boot_timeout_ms: 60_000,
        }
    }

    fn download_project() -> ProjectConfig {
        ProjectConfig {
            id: Some(7),
            name: "T153 SDK".to_owned(),
            host_id: 3,
            device_id: Some(5),
            sdk_dir: "/opt/sdk/t153".to_owned(),
            build_command: String::new(),
            firmware_path: "/opt/sdk/t153/out/*.img".to_owned(),
            local_download_dir: r"C:\Users\Public\Downloads".to_owned(),
            verify_sha256: true,
            local_gateway_port: 8765,
            remote_forward_port: 18765,
            mcp_server_name: "lynx-t153".to_owned(),
            ai_provider: AiProvider::Codex,
        }
    }

    #[test]
    fn accepts_matching_rockchip_flasher() {
        assert!(validate_device_vendor("development board", "rockchip", "openrockcli").is_ok());
    }

    #[test]
    fn serial_login_profile_rejects_rt_smart_auto_login_and_control_characters() {
        let mut device = board(1, "RK board");
        device.serial_console_type = "rt_smart".to_owned();
        device.serial_login_mode = "password".to_owned();
        device.serial_auto_login = true;
        assert_eq!(
            validate_serial_login_profile(&device).unwrap_err(),
            "SERIAL_AUTO_LOGIN_CONFIGURATION_INVALID"
        );

        device.serial_console_type = "linux".to_owned();
        device.serial_login_prompt = "login:\n".to_owned();
        assert_eq!(
            validate_serial_login_profile(&device).unwrap_err(),
            "SERIAL_LOGIN_FIELD_INVALID:loginPrompt"
        );
    }

    #[test]
    fn rejects_flasher_from_another_chip_vendor() {
        let error =
            validate_device_vendor("development board", "rockchip", "openixcli").unwrap_err();
        assert_eq!(
            error,
            "FLASHER_VENDOR_MISMATCH:rockchip requires openrockcli"
        );
    }

    #[test]
    fn accepts_matching_kendryte_flasher() {
        assert!(validate_device_vendor("K230 board", "kendryte", "k230").is_ok());
    }

    #[test]
    fn accepts_matching_nxp_uuu_flasher() {
        assert!(validate_device_vendor("i.MX6ULL board", "nxp", "uuu").is_ok());
    }

    #[test]
    fn accepts_matching_st_dfu_flasher() {
        assert!(validate_device_vendor("STM32MP157 EVB", "st", "dfu-util").is_ok());
    }

    #[test]
    fn rejects_duplicate_hardware_bindings_across_boards() {
        let existing = board(1, "board-a");

        let mut candidate = board(2, "board-b");
        candidate.serial_number = existing.serial_number.clone();
        candidate.serial_port = existing.serial_port.clone();
        assert!(
            validate_device_conflicts(&candidate, std::slice::from_ref(&existing))
                .unwrap_err()
                .contains("serial")
        );

        let mut candidate = board(2, "board-b");
        candidate.usb_binding = existing.usb_binding.clone();
        assert!(
            validate_device_conflicts(&candidate, std::slice::from_ref(&existing))
                .unwrap_err()
                .contains("flash USB")
        );

        let mut candidate = board(2, "board-b");
        candidate.adb_serial = existing.adb_serial.clone();
        assert!(validate_device_conflicts(&candidate, &[existing])
            .unwrap_err()
            .contains("ADB"));
    }

    #[test]
    fn lynx_power_channels_are_unique_per_controller_but_share_other_channels() {
        let mut channel_a = board(1, "board-a");
        channel_a.power_controller = "lynx-power:POWER-A".to_owned();
        channel_a.power_controller_serial = "POWER-A".to_owned();
        channel_a.power_channel = "4".to_owned();
        channel_a.power_profile_json = serde_json::json!({
            "type": "ch347_gpio", "deviceIndex": 0, "gpio": 4,
            "activeHigh": true, "roleMode": "singleRelay"
        })
        .to_string();

        let mut channel_b = board(2, "board-b");
        channel_b.power_controller = "lynx-power:POWER-A".to_owned();
        channel_b.power_controller_serial = "POWER-A".to_owned();
        channel_b.power_channel = "5".to_owned();
        channel_b.power_profile_json = serde_json::json!({
            "type": "ch347_gpio", "deviceIndex": 0, "gpio": 5,
            "activeHigh": true, "roleMode": "singleRelay"
        })
        .to_string();
        assert!(validate_device_conflicts(&channel_b, std::slice::from_ref(&channel_a)).is_ok());

        channel_b.power_channel = "4".to_owned();
        channel_b.power_profile_json = serde_json::json!({
            "type": "ch347_gpio", "deviceIndex": 0, "gpio": 4,
            "activeHigh": true, "roleMode": "singleRelay"
        })
        .to_string();
        let error = validate_device_conflicts(&channel_b, &[channel_a]).unwrap_err();
        assert!(error.contains("POWER_CHANNEL_ASSIGNED:GPIO4"));
        assert!(error.contains("board-a"));
    }

    #[test]
    fn windows_ch344_channels_with_shared_serial_use_distinct_com_ports() {
        let mut channel_a = board(1, "T153-channel-a");
        channel_a.serial_number = "0123456789".to_owned();
        channel_a.serial_port = "COM17".to_owned();
        channel_a.serial_interface_number = None;

        let mut channel_b = board(2, "AvaotaA1-channel-b");
        channel_b.serial_number = channel_a.serial_number.clone();
        channel_b.serial_port = "COM18".to_owned();
        channel_b.serial_interface_number = None;
        assert!(validate_device_conflicts(&channel_b, std::slice::from_ref(&channel_a)).is_ok());

        channel_b.serial_port = "com17".to_owned();
        assert!(validate_device_conflicts(&channel_b, &[channel_a])
            .unwrap_err()
            .contains("serial"));
    }

    #[test]
    fn rejects_vendor_that_conflicts_with_board_identity() {
        for (name, expected_vendor, wrong_vendor, wrong_flasher) in [
            ("RK3506 EVB", "rockchip", "allwinner", "openixcli"),
            ("T153MX OmniGate", "allwinner", "rockchip", "openrockcli"),
            ("NXP i.MX8MP", "nxp", "allwinner", "openixcli"),
            ("Kendryte K230", "kendryte", "rockchip", "openrockcli"),
        ] {
            assert_eq!(vendor_hint_from_name(name), Some(expected_vendor));
            assert!(validate_device_vendor(name, wrong_vendor, wrong_flasher)
                .unwrap_err()
                .starts_with("CHIP_VENDOR_IDENTITY_MISMATCH:"));
        }
        assert_eq!(vendor_hint_from_name("STM32MP157 EVB"), Some("st"));
    }

    #[test]
    fn matches_the_same_ssh_account_for_credential_reuse() {
        let mut left = host(AiProvider::Codex);
        left.address = " SDK.EXAMPLE.COM ".to_owned();
        left.username = " developer ".to_owned();
        let mut right = host(AiProvider::Codex);
        right.address = "sdk.example.com".to_owned();
        assert!(same_ssh_identity(&left, &right));
    }

    #[test]
    fn does_not_share_credentials_across_ssh_ports_or_users() {
        let left = host(AiProvider::Codex);
        let mut right = host(AiProvider::Codex);
        right.port = 2222;
        assert!(!same_ssh_identity(&left, &right));
        right.port = left.port;
        right.username = "root".to_owned();
        assert!(!same_ssh_identity(&left, &right));
    }

    #[test]
    fn download_validation_does_not_require_a_build_command() {
        assert!(validate_download_project(&download_project()).is_ok());
    }

    #[test]
    fn download_validation_requires_remote_and_local_paths() {
        let mut project = download_project();
        project.firmware_path = "relative.img".to_owned();
        assert!(validate_download_project(&project).is_err());
        project.firmware_path = "/opt/sdk/t153/out/image.img".to_owned();
        project.local_download_dir.clear();
        assert!(validate_download_project(&project).is_err());
    }

    #[test]
    fn sdk_channel_validation_does_not_require_ai_working_directory() {
        let mut project = download_project();
        project.sdk_dir.clear();
        project.firmware_path.clear();
        project.local_download_dir.clear();
        assert!(validate_project_config(&project).is_ok());
    }

    #[test]
    fn hashes_local_cache_files() {
        let path = std::env::temp_dir().join(new_task_id("lynx-sha256-test"));
        std::fs::write(&path, b"abc").unwrap();
        let hash = local_sha256(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn builds_safe_claude_code_command() {
        let command = build_ai_command(
            &host(AiProvider::ClaudeCode),
            "/srv/project",
            "fix user's test",
        )
        .unwrap();
        assert!(command.starts_with("bash -lc "));
        assert!(command.contains("claude"));
        assert!(command.contains("stream-json"));
        assert!(command.contains("acceptEdits"));
        assert!(command.contains("user"));
    }

    #[test]
    fn rejects_non_array_ai_arguments() {
        let mut value = host(AiProvider::Custom);
        value.ai_executable = "/opt/agent".to_owned();
        value.ai_args_json = "--unsafe".to_owned();
        assert!(resolved_ai_args(&value)
            .unwrap_err()
            .contains("JSON string array"));
    }

    #[test]
    fn decodes_split_claude_stream_text_without_duplicate_result() {
        let mut decoder = ClaudeStreamDecoder::default();
        let stream = "{\"type\":\"stream_event\",\"event\":{\"delta\":{\"text\":\"你好\"}}}\n\
                      {\"type\":\"result\",\"result\":\"你好\"}\n";
        let split = stream.find('你').unwrap() + 1;
        assert!(decoder.push(&stream.as_bytes()[..split]).is_empty());
        assert_eq!(decoder.push(&stream.as_bytes()[split..]), vec!["你好"]);
        assert!(decoder.finish().is_empty());
    }
}

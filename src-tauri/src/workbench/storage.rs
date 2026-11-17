use rusqlite::{params, Connection, OptionalExtension, Result, Row};
use sha2::{Digest, Sha256};

use super::types::{
    AiProvider, DeviceConfig, HostConfig, ProjectConfig, SshAuthType, TaskHistory,
    WorkflowStepHistory,
};

const MIGRATIONS: &[crate::database_migrations::Migration] = &[
    crate::database_migrations::Migration {
        version: 1,
        name: "legacy_v0_3_schema",
        apply: migrate_legacy_schema,
    },
    crate::database_migrations::Migration {
        version: 2,
        name: "add_stable_linux_board_uid",
        apply: migrate_stable_board_uid,
    },
    crate::database_migrations::Migration {
        version: 3,
        name: "add_serial_console_login_profile",
        apply: migrate_serial_console_login_profile,
    },
    crate::database_migrations::Migration {
        version: 4,
        name: "add_linux_usb_camera_binding",
        apply: migrate_linux_usb_camera_binding,
    },
];

pub fn migrate(
    db: &Connection,
) -> std::result::Result<(), crate::database_migrations::MigrationError> {
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    crate::database_migrations::apply(db, "workbench", MIGRATIONS)?;
    reconcile_startup_tasks(db)?;
    Ok(())
}

fn migrate_legacy_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS hosts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            address TEXT NOT NULL,
            port INTEGER NOT NULL,
            username TEXT NOT NULL,
            auth_type TEXT NOT NULL,
            private_key_path TEXT NOT NULL DEFAULT '',
            host_fingerprint TEXT NOT NULL DEFAULT '',
            default_work_dir TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            host_id INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
            sdk_dir TEXT NOT NULL,
            build_command TEXT NOT NULL,
            firmware_path TEXT NOT NULL,
            local_download_dir TEXT NOT NULL,
            verify_sha256 INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            serial_port TEXT NOT NULL DEFAULT '',
            serial_vid INTEGER NOT NULL DEFAULT 0,
            serial_pid INTEGER NOT NULL DEFAULT 0,
            serial_number TEXT NOT NULL DEFAULT '',
            baud_rate INTEGER NOT NULL DEFAULT 115200,
            flasher_kind TEXT NOT NULL DEFAULT 'openixcli',
            flasher_path TEXT NOT NULL DEFAULT '',
            flasher_args TEXT NOT NULL DEFAULT '',
            power_controller TEXT NOT NULL DEFAULT '',
            power_channel TEXT NOT NULL DEFAULT '',
            boot_channel TEXT NOT NULL DEFAULT '',
            reset_channel TEXT NOT NULL DEFAULT '',
            power_on_wait_ms INTEGER NOT NULL DEFAULT 2000
        );
        CREATE TABLE IF NOT EXISTS task_history (
            task_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            project_id INTEGER,
            device_id INTEGER,
            status TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            exit_code INTEGER,
            error TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS workflow_steps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            step_name TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            error TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS controller_actions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT,
            device_id INTEGER NOT NULL,
            action TEXT NOT NULL,
            parameters TEXT NOT NULL DEFAULT '',
            response TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        "#,
    )?;
    add_column(
        db,
        "devices",
        "power_controller_vid INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column(
        db,
        "devices",
        "power_controller_pid INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column(
        db,
        "devices",
        "power_controller_serial TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(
        db,
        "devices",
        "power_baud_rate INTEGER NOT NULL DEFAULT 115200",
    )?;
    add_column(db, "devices", "power_profile_json TEXT NOT NULL DEFAULT ''")?;
    add_column(
        db,
        "devices",
        "power_off_wait_ms INTEGER NOT NULL DEFAULT 1000",
    )?;
    add_column(
        db,
        "devices",
        "serial_reconnect_timeout_ms INTEGER NOT NULL DEFAULT 30000",
    )?;
    add_column(
        db,
        "devices",
        "boot_timeout_ms INTEGER NOT NULL DEFAULT 60000",
    )?;
    add_column(db, "devices", "serial_interface_number INTEGER")?;
    add_column(db, "devices", "power_controller_interface_number INTEGER")?;
    add_column(db, "hosts", "ai_provider TEXT NOT NULL DEFAULT 'codex'")?;
    add_column(db, "hosts", "ai_executable TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "hosts", "ai_args_json TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "devices", "usb_binding TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "devices", "adb_serial TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "devices", "board_image_path TEXT NOT NULL DEFAULT ''")?;
    add_column(
        db,
        "devices",
        "chip_vendor TEXT NOT NULL DEFAULT 'allwinner'",
    )?;
    add_column(
        db,
        "projects",
        "device_id INTEGER REFERENCES devices(id) ON DELETE SET NULL",
    )?;
    add_column(
        db,
        "projects",
        "local_gateway_port INTEGER NOT NULL DEFAULT 8765",
    )?;
    add_column(
        db,
        "projects",
        "remote_forward_port INTEGER NOT NULL DEFAULT 18765",
    )?;
    add_column(db, "projects", "mcp_server_name TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "projects", "ai_provider TEXT NOT NULL DEFAULT 'codex'")?;
    add_column(db, "task_history", "phase TEXT NOT NULL DEFAULT 'queued'")?;
    add_column(db, "task_history", "progress REAL")?;
    add_column(db, "task_history", "details_json TEXT NOT NULL DEFAULT ''")?;
    Ok(())
}

fn migrate_linux_usb_camera_binding(db: &Connection) -> Result<()> {
    add_column(db, "devices", "camera_binding TEXT NOT NULL DEFAULT ''")
}

fn reconcile_startup_tasks(db: &Connection) -> Result<()> {
    db.execute(
        "UPDATE task_history SET status='failed',finished_at=unixepoch(),\
         error=CASE WHEN error='' THEN 'APP_INTERRUPTED:LYNX process ended while the task was running; MCP session changes do not cancel tasks, and firmware downloads resume from the .part file when retried' ELSE error END \
         WHERE status='running'",
        [],
    )?;
    db.execute(
        "UPDATE task_history SET status='success',finished_at=COALESCE(finished_at,unixepoch()),\
         exit_code=0,error='',phase='complete',progress=100,\
         details_json=json_set(details_json,'$.phase','complete','$.progressPct',100,\
         '$.postFlashState','success','$.postFlashError',NULL) \
         WHERE kind='flash' AND status='failed' AND phase='post_flash_action' \
         AND COALESCE(json_extract(details_json,'$.storageType'),'')!='spinor' \
         AND json_extract(details_json,'$.verifyState')='success' \
         AND json_extract(details_json,'$.verifyErrorCode')=0 \
         AND json_extract(details_json,'$.postFlashAction')='reboot' \
         AND lower(COALESCE(json_extract(details_json,'$.postFlashError'),'')) LIKE '%device not found%'",
        [],
    )?;
    Ok(())
}

fn migrate_stable_board_uid(db: &Connection) -> Result<()> {
    add_column(db, "devices", "board_uid TEXT NOT NULL DEFAULT ''")?;
    let legacy_rows = {
        let mut statement = db.prepare(
            "SELECT id,name,chip_vendor,serial_vid,serial_pid,serial_number,usb_binding,adb_serial \
             FROM devices WHERE board_uid='' ORDER BY id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u16>(3)?,
                    row.get::<_, u16>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        rows
    };
    for (id, name, vendor, vid, pid, serial, usb_binding, adb_serial) in legacy_rows {
        let material = format!(
            "lynx-linux-board-v1\0{id}\0{name}\0{vendor}\0{vid}\0{pid}\0{serial}\0{usb_binding}\0{adb_serial}"
        );
        let digest = hex::encode(Sha256::digest(material.as_bytes()));
        let board_uid = format!("linux-{}", &digest[..32]);
        db.execute(
            "UPDATE devices SET board_uid=?1 WHERE id=?2 AND board_uid=''",
            params![board_uid, id],
        )?;
    }
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS devices_board_uid_unique ON devices(board_uid)",
        [],
    )?;
    Ok(())
}

fn migrate_serial_console_login_profile(db: &Connection) -> Result<()> {
    add_column(
        db,
        "devices",
        "serial_console_type TEXT NOT NULL DEFAULT 'auto'",
    )?;
    add_column(
        db,
        "devices",
        "serial_login_mode TEXT NOT NULL DEFAULT 'none'",
    )?;
    add_column(
        db,
        "devices",
        "serial_login_username TEXT NOT NULL DEFAULT 'root'",
    )?;
    add_column(
        db,
        "devices",
        "serial_login_prompt TEXT NOT NULL DEFAULT 'login:'",
    )?;
    add_column(
        db,
        "devices",
        "serial_password_prompt TEXT NOT NULL DEFAULT 'Password:'",
    )?;
    add_column(
        db,
        "devices",
        "serial_shell_prompt TEXT NOT NULL DEFAULT '#'",
    )?;
    add_column(
        db,
        "devices",
        "serial_auto_login INTEGER NOT NULL DEFAULT 0",
    )?;
    db.execute(
        "UPDATE devices SET serial_console_type='rt_smart',serial_auto_login=0 \
         WHERE chip_vendor='kendryte' AND serial_console_type='auto'",
        [],
    )?;
    Ok(())
}

fn add_column(db: &Connection, table: &str, definition: &str) -> Result<()> {
    let sql = format!("ALTER TABLE {table} ADD COLUMN {definition}");
    match db.execute(&sql, []) {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("duplicate column name") => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn list_hosts(db: &Connection) -> Result<Vec<HostConfig>> {
    let mut statement = db.prepare(
        "SELECT id,name,address,port,username,auth_type,private_key_path,\
         host_fingerprint,default_work_dir,ai_provider,ai_executable,ai_args_json \
         FROM hosts ORDER BY name",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(HostConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                address: row.get(2)?,
                port: row.get::<_, u16>(3)?,
                username: row.get(4)?,
                auth_type: SshAuthType::parse(&row.get::<_, String>(5)?),
                private_key_path: row.get(6)?,
                host_fingerprint: row.get(7)?,
                default_work_dir: row.get(8)?,
                ai_provider: AiProvider::parse(&row.get::<_, String>(9)?),
                ai_executable: row.get(10)?,
                ai_args_json: row.get(11)?,
            })
        })?
        .collect();
    rows
}

pub fn save_host(db: &Connection, host: &HostConfig) -> Result<i64> {
    if let Some(id) = host.id {
        db.execute(
            "UPDATE hosts SET name=?1,address=?2,port=?3,username=?4,auth_type=?5,\
             private_key_path=?6,host_fingerprint=?7,default_work_dir=?8,ai_provider=?9,\
             ai_executable=?10,ai_args_json=?11 WHERE id=?12",
            params![
                host.name,
                host.address,
                host.port,
                host.username,
                host.auth_type.as_str(),
                host.private_key_path,
                host.host_fingerprint,
                host.default_work_dir,
                host.ai_provider.as_str(),
                host.ai_executable,
                host.ai_args_json,
                id
            ],
        )?;
        Ok(id)
    } else {
        db.execute(
            "INSERT INTO hosts(name,address,port,username,auth_type,private_key_path,\
             host_fingerprint,default_work_dir,ai_provider,ai_executable,ai_args_json) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                host.name,
                host.address,
                host.port,
                host.username,
                host.auth_type.as_str(),
                host.private_key_path,
                host.host_fingerprint,
                host.default_work_dir,
                host.ai_provider.as_str(),
                host.ai_executable,
                host.ai_args_json
            ],
        )?;
        Ok(db.last_insert_rowid())
    }
}

pub fn list_projects(db: &Connection) -> Result<Vec<ProjectConfig>> {
    let mut statement = db.prepare(
        "SELECT id,name,host_id,sdk_dir,build_command,firmware_path,\
         local_download_dir,verify_sha256,device_id,local_gateway_port,\
         remote_forward_port,mcp_server_name,ai_provider FROM projects ORDER BY name",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(ProjectConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                host_id: row.get(2)?,
                sdk_dir: row.get(3)?,
                build_command: row.get(4)?,
                firmware_path: row.get(5)?,
                local_download_dir: row.get(6)?,
                verify_sha256: row.get(7)?,
                device_id: row.get(8)?,
                local_gateway_port: row.get(9)?,
                remote_forward_port: row.get(10)?,
                mcp_server_name: row.get(11)?,
                ai_provider: AiProvider::parse(&row.get::<_, String>(12)?),
            })
        })?
        .collect();
    rows
}

pub fn save_project(db: &Connection, project: &ProjectConfig) -> Result<i64> {
    if let Some(id) = project.id {
        db.execute(
            "UPDATE projects SET name=?1,host_id=?2,sdk_dir=?3,build_command=?4,\
             firmware_path=?5,local_download_dir=?6,verify_sha256=?7,device_id=?8,\
             local_gateway_port=?9,remote_forward_port=?10,mcp_server_name=?11,\
             ai_provider=?12 WHERE id=?13",
            params![
                project.name,
                project.host_id,
                project.sdk_dir,
                project.build_command,
                project.firmware_path,
                project.local_download_dir,
                project.verify_sha256,
                project.device_id,
                project.local_gateway_port,
                project.remote_forward_port,
                project.mcp_server_name,
                project.ai_provider.as_str(),
                id
            ],
        )?;
        Ok(id)
    } else {
        db.execute(
            "INSERT INTO projects(name,host_id,sdk_dir,build_command,firmware_path,\
             local_download_dir,verify_sha256,device_id,local_gateway_port,\
             remote_forward_port,mcp_server_name,ai_provider) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                project.name,
                project.host_id,
                project.sdk_dir,
                project.build_command,
                project.firmware_path,
                project.local_download_dir,
                project.verify_sha256,
                project.device_id,
                project.local_gateway_port,
                project.remote_forward_port,
                project.mcp_server_name,
                project.ai_provider.as_str()
            ],
        )?;
        Ok(db.last_insert_rowid())
    }
}

pub fn list_devices(db: &Connection) -> Result<Vec<DeviceConfig>> {
    let mut statement = db.prepare(
        "SELECT id,board_uid,name,chip_vendor,serial_port,serial_vid,serial_pid,serial_number,baud_rate,\
         flasher_kind,flasher_path,flasher_args,power_controller,power_channel,\
         boot_channel,reset_channel,power_on_wait_ms,power_controller_vid,\
         power_controller_pid,power_controller_serial,power_baud_rate,power_profile_json,\
         power_off_wait_ms,serial_reconnect_timeout_ms,boot_timeout_ms,\
         serial_interface_number,power_controller_interface_number,usb_binding,adb_serial,board_image_path,\
         serial_console_type,serial_login_mode,serial_login_username,serial_login_prompt,\
         serial_password_prompt,serial_shell_prompt,serial_auto_login,camera_binding \
         FROM devices ORDER BY name",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(DeviceConfig {
                id: row.get(0)?,
                board_uid: row.get(1)?,
                name: row.get(2)?,
                chip_vendor: row.get(3)?,
                serial_port: row.get(4)?,
                serial_vid: row.get(5)?,
                serial_pid: row.get(6)?,
                serial_number: row.get(7)?,
                baud_rate: row.get(8)?,
                flasher_kind: row.get(9)?,
                flasher_path: row.get(10)?,
                flasher_args: row.get(11)?,
                power_controller: row.get(12)?,
                power_channel: row.get(13)?,
                boot_channel: row.get(14)?,
                reset_channel: row.get(15)?,
                power_on_wait_ms: row.get(16)?,
                power_controller_vid: row.get(17)?,
                power_controller_pid: row.get(18)?,
                power_controller_serial: row.get(19)?,
                power_baud_rate: row.get(20)?,
                power_profile_json: row.get(21)?,
                power_off_wait_ms: row.get(22)?,
                serial_reconnect_timeout_ms: row.get(23)?,
                boot_timeout_ms: row.get(24)?,
                serial_interface_number: row.get(25)?,
                power_controller_interface_number: row.get(26)?,
                usb_binding: row.get(27)?,
                adb_serial: row.get(28)?,
                board_image_path: row.get(29)?,
                serial_console_type: row.get(30)?,
                serial_login_mode: row.get(31)?,
                serial_login_username: row.get(32)?,
                serial_login_prompt: row.get(33)?,
                serial_password_prompt: row.get(34)?,
                serial_shell_prompt: row.get(35)?,
                serial_auto_login: row.get(36)?,
                camera_binding: row.get(37)?,
            })
        })?
        .collect();
    rows
}

pub fn save_device(db: &Connection, device: &DeviceConfig) -> Result<i64> {
    if let Some(id) = device.id {
        db.execute(
            "UPDATE devices SET name=?1,chip_vendor=?2,serial_port=?3,serial_vid=?4,serial_pid=?5,\
             serial_number=?6,baud_rate=?7,flasher_kind=?8,flasher_path=?9,\
             flasher_args=?10,power_controller=?11,power_channel=?12,boot_channel=?13,\
             reset_channel=?14,power_on_wait_ms=?15,power_controller_vid=?16,\
             power_controller_pid=?17,power_controller_serial=?18,power_baud_rate=?19,\
             power_profile_json=?20,power_off_wait_ms=?21,serial_reconnect_timeout_ms=?22,\
             boot_timeout_ms=?23,serial_interface_number=?24,\
             power_controller_interface_number=?25,usb_binding=?26,adb_serial=?27,\
             board_image_path=?28,serial_console_type=?29,serial_login_mode=?30,\
             serial_login_username=?31,serial_login_prompt=?32,serial_password_prompt=?33,\
             serial_shell_prompt=?34,serial_auto_login=?35,camera_binding=?36 WHERE id=?37",
            params![
                device.name,
                device.chip_vendor,
                device.serial_port,
                device.serial_vid,
                device.serial_pid,
                device.serial_number,
                device.baud_rate,
                device.flasher_kind,
                device.flasher_path,
                device.flasher_args,
                device.power_controller,
                device.power_channel,
                device.boot_channel,
                device.reset_channel,
                device.power_on_wait_ms,
                device.power_controller_vid,
                device.power_controller_pid,
                device.power_controller_serial,
                device.power_baud_rate,
                device.power_profile_json,
                device.power_off_wait_ms,
                device.serial_reconnect_timeout_ms,
                device.boot_timeout_ms,
                device.serial_interface_number,
                device.power_controller_interface_number,
                device.usb_binding,
                device.adb_serial,
                device.board_image_path,
                device.serial_console_type,
                device.serial_login_mode,
                device.serial_login_username,
                device.serial_login_prompt,
                device.serial_password_prompt,
                device.serial_shell_prompt,
                device.serial_auto_login,
                device.camera_binding,
                id
            ],
        )?;
        Ok(id)
    } else {
        let board_uid = new_board_uid(db)?;
        db.execute(
            "INSERT INTO devices(board_uid,name,chip_vendor,serial_port,serial_vid,serial_pid,serial_number,\
             baud_rate,flasher_kind,flasher_path,flasher_args,power_controller,\
             power_channel,boot_channel,reset_channel,power_on_wait_ms,power_controller_vid,\
             power_controller_pid,power_controller_serial,power_baud_rate,power_profile_json,\
             power_off_wait_ms,serial_reconnect_timeout_ms,boot_timeout_ms,\
             serial_interface_number,power_controller_interface_number,usb_binding,adb_serial,board_image_path,\
             serial_console_type,serial_login_mode,serial_login_username,serial_login_prompt,\
             serial_password_prompt,serial_shell_prompt,serial_auto_login,camera_binding)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37)",
            params![
                board_uid,
                device.name,
                device.chip_vendor,
                device.serial_port,
                device.serial_vid,
                device.serial_pid,
                device.serial_number,
                device.baud_rate,
                device.flasher_kind,
                device.flasher_path,
                device.flasher_args,
                device.power_controller,
                device.power_channel,
                device.boot_channel,
                device.reset_channel,
                device.power_on_wait_ms
                ,device.power_controller_vid
                ,device.power_controller_pid
                ,device.power_controller_serial
                ,device.power_baud_rate
                ,device.power_profile_json
                ,device.power_off_wait_ms
                ,device.serial_reconnect_timeout_ms
                ,device.boot_timeout_ms
                ,device.serial_interface_number
                ,device.power_controller_interface_number
                ,device.usb_binding
                ,device.adb_serial
                ,device.board_image_path
                ,device.serial_console_type
                ,device.serial_login_mode
                ,device.serial_login_username
                ,device.serial_login_prompt
                ,device.serial_password_prompt
                ,device.serial_shell_prompt
                ,device.serial_auto_login
                ,device.camera_binding
            ],
        )?;
        Ok(db.last_insert_rowid())
    }
}

fn new_board_uid(db: &Connection) -> Result<String> {
    loop {
        let candidate = format!("linux-{:032x}", rand::random::<u128>());
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE board_uid=?1)",
            [&candidate],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(candidate);
        }
    }
}

pub fn list_tasks(db: &Connection, limit: u32) -> Result<Vec<TaskHistory>> {
    let mut statement = db.prepare(
        "SELECT task_id,kind,project_id,device_id,status,started_at,finished_at,exit_code,error,\
         phase,progress,details_json \
         FROM task_history ORDER BY started_at DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit.clamp(1, 500)], task_from_row)?
        .collect();
    rows
}

/// Bound before LIMIT, so busy unrelated channels cannot hide this board's task.
pub fn list_channel_tasks(
    db: &Connection,
    project_id: i64,
    device_id: i64,
) -> Result<Vec<TaskHistory>> {
    let mut statement = db.prepare(
        "SELECT task_id,kind,project_id,device_id,status,started_at,finished_at,exit_code,error,\
         phase,progress,details_json FROM task_history \
         WHERE (project_id=?1 AND (device_id IS NULL OR device_id=?2)) \
            OR (project_id IS NULL AND device_id=?2) \
         ORDER BY started_at DESC, task_id DESC LIMIT 20",
    )?;
    let rows = statement
        .query_map(params![project_id, device_id], task_from_row)?
        .collect();
    rows
}

pub fn task_by_id(db: &Connection, task_id: &str) -> Result<Option<TaskHistory>> {
    db.query_row(
        "SELECT task_id,kind,project_id,device_id,status,started_at,finished_at,exit_code,error,\
         phase,progress,details_json FROM task_history WHERE task_id=?1",
        [task_id],
        task_from_row,
    )
    .optional()
}

pub fn transition_task_status(
    db: &Connection,
    task_id: &str,
    status: &str,
    exit_code: Option<i32>,
    error: &str,
) -> Result<usize> {
    let current = db
        .query_row(
            "SELECT status FROM task_history WHERE task_id=?1",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(current) = current else {
        return Ok(0);
    };
    crate::task_lifecycle::validate_workbench_transition(&current, status)
        .map_err(|code| rusqlite::Error::InvalidParameterName(code.to_owned()))?;
    if current == status {
        return Ok(1);
    }
    db.execute(
        "UPDATE task_history SET status=?1,finished_at=unixepoch(),exit_code=?2,error=?3,
         phase=CASE WHEN ?1='success' THEN 'complete' ELSE phase END,
         progress=CASE WHEN ?1='success' THEN 100 ELSE progress END
         WHERE task_id=?4 AND status=?5",
        params![status, exit_code, error, task_id, current],
    )
}

fn task_from_row(row: &Row<'_>) -> Result<TaskHistory> {
    let details_json: String = row.get(11)?;
    Ok(TaskHistory {
        task_id: row.get(0)?,
        kind: row.get(1)?,
        project_id: row.get(2)?,
        device_id: row.get(3)?,
        status: row.get(4)?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
        exit_code: row.get(7)?,
        error: row.get(8)?,
        phase: row.get(9)?,
        progress_pct: row.get(10)?,
        details: if details_json.trim().is_empty() {
            None
        } else {
            serde_json::from_str(&details_json).ok()
        },
    })
}

pub fn list_workflow_steps(db: &Connection, task_id: &str) -> Result<Vec<WorkflowStepHistory>> {
    let mut statement = db.prepare(
        "SELECT id,task_id,step_name,status,started_at,finished_at,error \
         FROM workflow_steps WHERE task_id=?1 ORDER BY id",
    )?;
    let rows = statement
        .query_map([task_id], |row| {
            Ok(WorkflowStepHistory {
                id: row.get(0)?,
                task_id: row.get(1)?,
                step_name: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                error: row.get(6)?,
            })
        })?
        .collect();
    rows
}

pub fn delete_config(db: &Connection, kind: &str, id: i64) -> Result<()> {
    if kind == "device" {
        db.execute("DELETE FROM projects WHERE device_id=?1", [id])?;
    }
    let table = match kind {
        "host" => "hosts",
        "project" => "projects",
        "device" => "devices",
        _ => return Err(rusqlite::Error::InvalidParameterName(kind.to_owned())),
    };
    db.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])?;
    Ok(())
}

pub fn host_by_id(db: &Connection, id: i64) -> Result<HostConfig> {
    list_hosts(db)?
        .into_iter()
        .find(|value| value.id == Some(id))
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn project_by_id(db: &Connection, id: i64) -> Result<ProjectConfig> {
    list_projects(db)?
        .into_iter()
        .find(|value| value.id == Some(id))
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn device_by_id(db: &Connection, id: i64) -> Result<DeviceConfig> {
    list_devices(db)?
        .into_iter()
        .find(|value| value.id == Some(id))
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

#[cfg(test)]
mod device_tests {
    use super::*;

    #[test]
    fn rockchip_device_round_trips_through_migrated_database() {
        let db = Connection::open_in_memory().expect("open database");
        migrate(&db).expect("migrate database");
        let device = DeviceConfig {
            id: None,
            board_uid: String::new(),
            name: "RK3588 board".to_owned(),
            board_image_path: "/tmp/rk3588.png".to_owned(),
            chip_vendor: "rockchip".to_owned(),
            serial_port: String::new(),
            serial_vid: 0,
            serial_pid: 0,
            serial_number: String::new(),
            usb_binding: "1:2.3".to_owned(),
            camera_binding: "0".to_owned(),
            adb_serial: String::new(),
            serial_interface_number: None,
            baud_rate: 115_200,
            serial_console_type: "linux".to_owned(),
            serial_login_mode: "password".to_owned(),
            serial_login_username: "root".to_owned(),
            serial_login_prompt: "login:".to_owned(),
            serial_password_prompt: "Password:".to_owned(),
            serial_shell_prompt: "#".to_owned(),
            serial_auto_login: true,
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
        };
        let id = save_device(&db, &device).expect("save device");
        let loaded = device_by_id(&db, id).expect("load device");
        assert_eq!(loaded.chip_vendor, "rockchip");
        assert_eq!(loaded.flasher_kind, "openrockcli");
        assert_eq!(loaded.usb_binding, "1:2.3");
        assert_eq!(loaded.camera_binding, "0");
        assert_eq!(loaded.board_image_path, "/tmp/rk3588.png");
        assert_eq!(loaded.serial_console_type, "linux");
        assert_eq!(loaded.serial_login_mode, "password");
        assert!(loaded.serial_auto_login);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_ledger_adopts_legacy_schema_once() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE hosts(\
             id INTEGER PRIMARY KEY,name TEXT NOT NULL,address TEXT NOT NULL,port INTEGER NOT NULL,\
             username TEXT NOT NULL,auth_type TEXT NOT NULL,private_key_path TEXT NOT NULL DEFAULT '',\
             host_fingerprint TEXT NOT NULL DEFAULT '',default_work_dir TEXT NOT NULL DEFAULT '');",
        )
        .unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        let ledger: Vec<(i64, String)> = db
            .prepare("SELECT version,name FROM lynx_schema_migration ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            ledger,
            vec![
                (1, "legacy_v0_3_schema".into()),
                (2, "add_stable_linux_board_uid".into()),
                (3, "add_serial_console_login_profile".into()),
                (4, "add_linux_usb_camera_binding".into())
            ]
        );
        assert!(db
            .query_row(
                "SELECT 1 FROM pragma_table_info('hosts') WHERE name='ai_provider'",
                [],
                |_| Ok(())
            )
            .is_ok());
        assert!(db
            .query_row(
                "SELECT 1 FROM pragma_table_info('devices') WHERE name='serial_console_type'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .is_ok());
    }

    #[test]
    fn serial_profile_migration_marks_existing_k230_as_rt_smart() {
        let db = Connection::open_in_memory().unwrap();
        migrate_legacy_schema(&db).unwrap();
        db.execute(
            "INSERT INTO devices(name,chip_vendor) VALUES('K230 board','kendryte')",
            [],
        )
        .unwrap();
        migrate_serial_console_login_profile(&db).unwrap();
        let profile: (String, bool) = db
            .query_row(
                "SELECT serial_console_type,serial_auto_login FROM devices WHERE name='K230 board'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(profile, ("rt_smart".to_owned(), false));
    }

    #[test]
    fn legacy_board_uid_backfill_is_deterministic_and_non_empty() {
        fn migrated_uid() -> String {
            let db = Connection::open_in_memory().unwrap();
            migrate_legacy_schema(&db).unwrap();
            db.execute(
                "INSERT INTO devices(\
                 id,name,chip_vendor,serial_vid,serial_pid,serial_number,usb_binding,adb_serial)\
                 VALUES(7,'T153MX-OminiGate','allwinner',6790,30084,'SN-T153','1:7','t153-adb')",
                [],
            )
            .unwrap();
            migrate(&db).unwrap();
            db.query_row("SELECT board_uid FROM devices WHERE id=7", [], |row| {
                row.get(0)
            })
            .unwrap()
        }

        let first = migrated_uid();
        let second = migrated_uid();
        assert_eq!(first, second);
        assert!(first.starts_with("linux-"));
        assert_eq!(first.len(), 38);
        assert!(first[6..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn board_uid_is_generated_once_and_cannot_be_changed_by_edit_payload() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let mut device = test_device(None, "T153");
        device.board_uid = "linux-client-must-not-choose".into();
        let id = save_device(&db, &device).unwrap();
        let created = device_by_id(&db, id).unwrap();
        assert!(created.board_uid.starts_with("linux-"));
        assert_ne!(created.board_uid, device.board_uid);

        let mut edited = created.clone();
        edited.name = "T153 renamed".into();
        edited.board_uid = "linux-client-must-not-replace".into();
        save_device(&db, &edited).unwrap();
        let loaded = device_by_id(&db, id).unwrap();
        assert_eq!(loaded.board_uid, created.board_uid);
        assert_eq!(loaded.name, "T153 renamed");
    }

    #[test]
    fn deleting_and_recreating_board_does_not_reuse_uid() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let first_id = save_device(&db, &test_device(None, "T153")).unwrap();
        let first_uid = device_by_id(&db, first_id).unwrap().board_uid;
        delete_config(&db, "device", first_id).unwrap();
        let second_id = save_device(&db, &test_device(None, "T153")).unwrap();
        let second_uid = device_by_id(&db, second_id).unwrap().board_uid;
        assert_ne!(first_uid, second_uid);
    }

    #[test]
    fn channel_task_history_filters_scope_before_limit_and_retains_diagnostics() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        for i in 0..60 {
            db.execute("INSERT INTO task_history(task_id,kind,project_id,device_id,status,started_at,phase,details_json) VALUES(?1,'flash',?2,?3,'failed',?4,'verify',?5)",
                params![format!("task-{i:03}"), if i < 30 { None } else { Some(99) }, if i < 30 { 6 } else { 7 }, i,
                    r#"{"outputTail":["Error: ROCKCHIP_VERIFY_FAILED backup GPT"],"verifyMethod":"device-checksum"}"#]).unwrap();
        }
        // A saved project rebound to a different board must not expose old board tasks.
        db.execute("INSERT INTO task_history(task_id,kind,project_id,device_id,status,started_at,phase) VALUES('old-board','flash',6,7,'failed',100,'verify')", []).unwrap();
        let tasks = list_channel_tasks(&db, 6, 6).unwrap();
        assert_eq!(tasks.len(), 20);
        assert_eq!(tasks[0].task_id, "task-029");
        assert_eq!(tasks[19].task_id, "task-010");
        assert!(tasks.iter().all(|task| task.device_id == Some(6)));
        assert_eq!(
            tasks[0].details.as_ref().unwrap()["verifyMethod"],
            "device-checksum"
        );
        assert!(list_channel_tasks(&db, 88, 88).unwrap().is_empty());
        db.execute("INSERT INTO task_history(task_id,kind,project_id,status,started_at,phase) VALUES('download','download',6,'success',101,'complete')", []).unwrap();
        assert_eq!(
            list_channel_tasks(&db, 6, 6).unwrap()[0].task_id,
            "download"
        );
    }

    #[test]
    fn task_terminal_state_rejects_late_callback_overwrite() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO task_history(task_id,kind,status,started_at,phase)
             VALUES('task-late','workflow','running',unixepoch(),'workflow')",
            [],
        )
        .unwrap();
        assert_eq!(
            transition_task_status(&db, "task-late", "success", Some(0), "").unwrap(),
            1
        );
        let finished_at: i64 = db
            .query_row(
                "SELECT finished_at FROM task_history WHERE task_id='task-late'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let error = transition_task_status(&db, "task-late", "failed", Some(1), "late callback")
            .unwrap_err()
            .to_string();
        assert!(error.contains("TASK_STATE_TRANSITION_INVALID"));
        let saved: (String, i64, String) = db
            .query_row(
                "SELECT status,finished_at,error FROM task_history WHERE task_id='task-late'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(saved, ("success".to_owned(), finished_at, String::new()));
    }

    fn test_device(id: Option<i64>, name: &str) -> DeviceConfig {
        DeviceConfig {
            id,
            board_uid: String::new(),
            name: name.into(),
            board_image_path: String::new(),
            chip_vendor: "allwinner".into(),
            serial_port: String::new(),
            serial_vid: 0,
            serial_pid: 0,
            serial_number: String::new(),
            usb_binding: String::new(),
            camera_binding: String::new(),
            adb_serial: String::new(),
            serial_interface_number: None,
            baud_rate: 115_200,
            serial_console_type: "auto".to_owned(),
            serial_login_mode: "none".to_owned(),
            serial_login_username: "root".to_owned(),
            serial_login_prompt: "login:".to_owned(),
            serial_password_prompt: "Password:".to_owned(),
            serial_shell_prompt: "#".to_owned(),
            serial_auto_login: false,
            flasher_kind: "openixcli".into(),
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

    #[test]
    fn migration_marks_interrupted_tasks_failed() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO task_history(task_id,kind,status,started_at) \
             VALUES('old','workflow','running',1)",
            [],
        )
        .unwrap();
        migrate(&db).unwrap();
        let (status, error): (String, String) = db
            .query_row(
                "SELECT status,error FROM task_history WHERE task_id='old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert!(error.starts_with("APP_INTERRUPTED:"));
    }

    #[test]
    fn migration_reconciles_verified_flash_when_reboot_removes_fel_device() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO task_history(\
             task_id,kind,status,started_at,finished_at,exit_code,error,phase,progress,details_json)\
             VALUES('rebooted','flash','failed',1,2,1,'openixcli exited with 1',\
             'post_flash_action',98,\
             '{\"verifyState\":\"success\",\"verifyErrorCode\":0,\"postFlashAction\":\"reboot\",\"postFlashState\":\"failed\",\"postFlashError\":\"Failed to open device: Device not found\"}')",
            [],
        )
        .unwrap();
        migrate(&db).unwrap();
        let (status, phase, progress, error): (String, String, f64, String) = db
            .query_row(
                "SELECT status,phase,progress,error FROM task_history WHERE task_id='rebooted'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (status.as_str(), phase.as_str(), progress, error.as_str()),
            ("success", "complete", 100.0, "")
        );
    }

    #[test]
    fn migration_never_reclassifies_spinor_reboot_failure_as_success() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO task_history(task_id,kind,status,started_at,phase,error,details_json) VALUES(?1,'flash','failed',1,'post_flash_action','reboot failed',?2)",
            params!["spinor", serde_json::json!({
                "storageType":"spinor", "verifyState":"success", "verifyErrorCode":0,
                "postFlashAction":"reboot", "postFlashState":"failed",
                "postFlashError":"Device not found", "physicalVerifyState":"not_verified",
                "bootVerifyState":"not_verified"
            }).to_string()],
        ).unwrap();
        migrate(&db).unwrap();
        let task = list_tasks(&db, 10).unwrap().remove(0);
        assert_eq!(task.status, "failed");
        assert_eq!(task.error, "reboot failed");
        assert_eq!(task.details.unwrap()["postFlashState"], "failed");
    }

    #[test]
    fn flash_telemetry_round_trips_from_task_history() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO task_history(\
             task_id,kind,device_id,status,started_at,phase,progress,details_json)\
             VALUES('flash-1','flash',7,'running',1,'write',42.5,\
             '{\"currentPartition\":\"rootfs\",\"usbSpeed\":\"high\"}')",
            [],
        )
        .unwrap();
        let task = list_tasks(&db, 10).unwrap().remove(0);
        assert_eq!(task.phase, "write");
        assert_eq!(task.progress_pct, Some(42.5));
        assert_eq!(task.details.unwrap()["usbSpeed"], "high");
    }

    #[test]
    fn migration_adds_ai_channel_defaults_to_existing_hosts() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE hosts (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                address TEXT NOT NULL,
                port INTEGER NOT NULL,
                username TEXT NOT NULL,
                auth_type TEXT NOT NULL,
                private_key_path TEXT NOT NULL DEFAULT '',
                host_fingerprint TEXT NOT NULL DEFAULT '',
                default_work_dir TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO hosts(name,address,port,username,auth_type)
            VALUES('Ubuntu','127.0.0.1',22,'dev','private_key');",
        )
        .unwrap();
        migrate(&db).unwrap();
        let host = list_hosts(&db).unwrap().remove(0);
        assert!(matches!(host.ai_provider, AiProvider::Codex));
        assert!(host.ai_executable.is_empty());
        assert!(host.ai_args_json.is_empty());
    }

    #[test]
    fn deepseek_harness_project_provider_round_trips() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO hosts(name,address,port,username,auth_type)
             VALUES('Ubuntu','127.0.0.1',22,'dev','private_key')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO projects(
                name,host_id,sdk_dir,build_command,firmware_path,local_download_dir,ai_provider
             ) VALUES('T527 SDK',1,'/opt/t527','','','','deepseek_harness')",
            [],
        )
        .unwrap();
        let project = list_projects(&db).unwrap().remove(0);
        assert!(matches!(project.ai_provider, AiProvider::DeepseekHarness));
        assert_eq!(project.ai_provider.as_str(), "deepseek_harness");
        assert_eq!(project.ai_provider.default_executable(), "dsh");
    }

    #[test]
    fn deleting_device_removes_its_projects_but_keeps_shared_host() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO hosts(name,address,port,username,auth_type)
             VALUES('Shared','127.0.0.1',22,'dev','private_key')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO devices(name) VALUES('RK board')", [])
            .unwrap();
        db.execute(
            "INSERT INTO projects(
                name,host_id,device_id,sdk_dir,build_command,firmware_path,local_download_dir
             ) VALUES('RK SDK',1,1,'','','','')",
            [],
        )
        .unwrap();

        delete_config(&db, "device", 1).unwrap();

        let project_count: i64 = db
            .query_row("SELECT count(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        let host_count: i64 = db
            .query_row("SELECT count(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(project_count, 0);
        assert_eq!(host_count, 1);
    }
}

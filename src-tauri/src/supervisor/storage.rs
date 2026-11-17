use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::credentials;
use super::types::{
    AiProviderConfig, AutomaticTask, HumanInterventionRequest, SaveAiProviderRequest,
    SupervisorAuditRecord, SupervisorDecision, SupervisorOverview, SupervisorPolicy,
    SupervisorRound, SupervisorRun,
};

const MIGRATIONS: &[crate::database_migrations::Migration] = &[
    crate::database_migrations::Migration {
        version: 1,
        name: "legacy_v0_3_schema",
        apply: migrate_legacy_schema,
    },
    crate::database_migrations::Migration {
        version: 2,
        name: "add_stable_board_identity_mirror",
        apply: migrate_stable_board_identity_mirror,
    },
];

pub fn migrate(
    db: &Connection,
) -> std::result::Result<(), crate::database_migrations::MigrationError> {
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    crate::database_migrations::apply(db, "supervisor", MIGRATIONS)
}

fn migrate_legacy_schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS ai_provider_config (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            preset TEXT NOT NULL DEFAULT 'custom',
            base_url TEXT NOT NULL,
            model TEXT NOT NULL,
            timeout_seconds INTEGER NOT NULL DEFAULT 120,
            max_tokens INTEGER,
            task_cost_limit REAL,
            input_cost_per_million REAL,
            output_cost_per_million REAL,
            enabled INTEGER NOT NULL DEFAULT 1,
            is_default INTEGER NOT NULL DEFAULT 0,
            last_test_status TEXT NOT NULL DEFAULT 'untested',
            last_test_message TEXT NOT NULL DEFAULT '',
            last_test_at INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS ai_provider_one_default
            ON ai_provider_config(is_default) WHERE is_default = 1;
        CREATE TABLE IF NOT EXISTS supervisor_policy (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            provider_id INTEGER REFERENCES ai_provider_config(id) ON DELETE SET NULL,
            max_attempts INTEGER NOT NULL DEFAULT 6,
            max_duration_seconds INTEGER NOT NULL DEFAULT 1800,
            max_same_error INTEGER NOT NULL DEFAULT 2,
            max_power_cycles INTEGER NOT NULL DEFAULT 3,
            max_flash_retries INTEGER NOT NULL DEFAULT 2,
            wait_device_seconds INTEGER NOT NULL DEFAULT 30,
            cost_limit REAL,
            allowed_actions_json TEXT NOT NULL,
            approval_actions_json TEXT NOT NULL,
            board_scope_json TEXT NOT NULL DEFAULT '[]',
            success_summary INTEGER NOT NULL DEFAULT 1,
            notification_mode TEXT NOT NULL DEFAULT 'desktop'
        );
        CREATE TABLE IF NOT EXISTS supervisor_run (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            board_id TEXT NOT NULL,
            policy_id INTEGER NOT NULL REFERENCES supervisor_policy(id),
            status TEXT NOT NULL,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            power_cycle_count INTEGER NOT NULL DEFAULT 0,
            flash_retry_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost REAL NOT NULL DEFAULT 0,
            final_reason TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS supervisor_round (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL REFERENCES supervisor_run(id) ON DELETE CASCADE,
            round_index INTEGER NOT NULL,
            failure_fingerprint TEXT NOT NULL,
            redacted_context_json TEXT NOT NULL,
            decision_json TEXT NOT NULL,
            schema_result TEXT NOT NULL,
            policy_result TEXT NOT NULL,
            action_result TEXT NOT NULL,
            verification_result TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost REAL NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE(run_id, round_index)
        );
        CREATE TABLE IF NOT EXISTS supervisor_human_request (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL REFERENCES supervisor_run(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'waiting',
            reason TEXT NOT NULL,
            instruction TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            resolution TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS supervisor_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT,
            board_id TEXT NOT NULL DEFAULT '',
            provider_id INTEGER REFERENCES ai_provider_config(id) ON DELETE SET NULL,
            event_type TEXT NOT NULL,
            result TEXT NOT NULL,
            redacted_input_json TEXT NOT NULL DEFAULT '{}',
            output_json TEXT NOT NULL DEFAULT '{}',
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost REAL NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS supervisor_automatic_task (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 0,
            trigger_type TEXT NOT NULL,
            trigger_json TEXT NOT NULL,
            board_id TEXT NOT NULL,
            task_template_json TEXT NOT NULL,
            policy_id INTEGER REFERENCES supervisor_policy(id) ON DELETE SET NULL,
            success_conditions_json TEXT NOT NULL DEFAULT '[]',
            next_run_at INTEGER,
            last_run_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "#,
    )?;
    // Development builds may already have created the v0.3 database before
    // provider pricing was added. Keep that database forward-compatible.
    for (name, declaration) in [
        ("input_cost_per_million", "REAL"),
        ("output_cost_per_million", "REAL"),
    ] {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ai_provider_config') WHERE name=?1)",
            [name],
            |row| row.get(0),
        )?;
        if !exists {
            db.execute(
                &format!("ALTER TABLE ai_provider_config ADD COLUMN {name} {declaration}"),
                [],
            )?;
        }
    }
    Ok(())
}

fn migrate_stable_board_identity_mirror(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS supervisor_board_identity (
            legacy_board_id TEXT PRIMARY KEY,
            board_uid TEXT NOT NULL UNIQUE,
            synced_at INTEGER NOT NULL
        );",
    )?;
    add_column(
        db,
        "supervisor_policy",
        "board_uid_scope_json TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column(
        db,
        "supervisor_automatic_task",
        "board_uid TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(db, "supervisor_run", "board_uid TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "supervisor_audit", "board_uid TEXT NOT NULL DEFAULT ''")?;
    Ok(())
}

fn add_column(db: &Connection, table: &str, definition: &str) -> rusqlite::Result<()> {
    let sql = format!("ALTER TABLE {table} ADD COLUMN {definition}");
    match db.execute(&sql, []) {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("duplicate column name") => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn sync_board_identities(
    supervisor: &mut Connection,
    workbench: &Connection,
) -> rusqlite::Result<usize> {
    let devices = crate::workbench::storage::list_devices(workbench)?;
    let transaction = supervisor.transaction()?;
    let mut synced = 0;
    for device in devices {
        let (Some(id), board_uid) = (device.id, device.board_uid.trim()) else {
            continue;
        };
        if board_uid.is_empty() {
            continue;
        }
        let legacy_board_id = format!("linux:{id}");
        transaction.execute(
            "INSERT INTO supervisor_board_identity(legacy_board_id,board_uid,synced_at)
             VALUES(?1,?2,unixepoch())
             ON CONFLICT(legacy_board_id) DO NOTHING",
            params![legacy_board_id, board_uid],
        )?;
        let stored_uid: String = transaction.query_row(
            "SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=?1",
            [&legacy_board_id],
            |row| row.get(0),
        )?;
        if stored_uid != board_uid {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "SUPERVISOR_BOARD_IDENTITY_CONFLICT:{legacy_board_id}"
            )));
        }
        transaction.execute(
            "UPDATE supervisor_board_identity SET synced_at=unixepoch() WHERE legacy_board_id=?1",
            [&legacy_board_id],
        )?;
        synced += 1;
    }
    transaction.execute(
        "UPDATE supervisor_automatic_task SET board_uid=COALESCE((
            SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=board_id
         ),board_uid) WHERE board_uid=''",
        [],
    )?;
    transaction.execute(
        "UPDATE supervisor_run SET board_uid=COALESCE((
            SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=board_id
         ),board_uid) WHERE board_uid=''",
        [],
    )?;
    transaction.execute(
        "UPDATE supervisor_audit SET board_uid=COALESCE((
            SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=board_id
         ),board_uid) WHERE board_uid=''",
        [],
    )?;
    let policies = {
        let mut statement = transaction.prepare(
            "SELECT id,board_scope_json FROM supervisor_policy WHERE board_uid_scope_json='[]'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, scope_json) in policies {
        let scopes: Vec<String> = serde_json::from_str(&scope_json).unwrap_or_default();
        let uid_scopes = board_uids_for_scopes(&transaction, &scopes)?;
        transaction.execute(
            "UPDATE supervisor_policy SET board_uid_scope_json=?1 WHERE id=?2",
            params![
                serde_json::to_string(&uid_scopes).unwrap_or_else(|_| "[]".to_owned()),
                id
            ],
        )?;
    }
    transaction.commit()?;
    Ok(synced)
}

fn board_uids_for_scopes(db: &Connection, scopes: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut board_uids = Vec::new();
    for scope in scopes {
        let board_uid = db
            .query_row(
                "SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=?1",
                [scope],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(board_uid) = board_uid {
            if !board_uids.contains(&board_uid) {
                board_uids.push(board_uid);
            }
        }
    }
    Ok(board_uids)
}

fn board_uid_for_scope(db: &Connection, scope: &str) -> rusqlite::Result<String> {
    Ok(db
        .query_row(
            "SELECT board_uid FROM supervisor_board_identity WHERE legacy_board_id=?1",
            [scope],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod schema_tests {
    use super::*;

    #[test]
    fn migration_ledger_adopts_legacy_provider_schema_once() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE ai_provider_config(\
             id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,preset TEXT NOT NULL,\
             base_url TEXT NOT NULL,model TEXT NOT NULL,timeout_seconds INTEGER NOT NULL,\
             max_tokens INTEGER,task_cost_limit REAL,enabled INTEGER NOT NULL,is_default INTEGER NOT NULL,\
             last_test_status TEXT NOT NULL,last_test_message TEXT NOT NULL,last_test_at INTEGER);",
        )
        .unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM lynx_schema_migration", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
        for column in ["input_cost_per_million", "output_cost_per_million"] {
            assert!(db
                .query_row(
                    "SELECT 1 FROM pragma_table_info('ai_provider_config') WHERE name=?1",
                    [column],
                    |_| Ok(())
                )
                .is_ok());
        }
    }

    #[test]
    fn stable_board_identity_sync_backfills_and_dual_writes_without_changing_legacy_scope() {
        let mut supervisor = Connection::open_in_memory().unwrap();
        migrate(&supervisor).unwrap();
        let workbench = Connection::open_in_memory().unwrap();
        crate::workbench::storage::migrate(&workbench).unwrap();
        workbench
            .execute(
                "INSERT INTO devices(board_uid,name) VALUES(?1,'T153')",
                ["linux-11111111111111111111111111111111"],
            )
            .unwrap();
        let policy_id = save_policy(
            &supervisor,
            &SupervisorPolicy {
                id: None,
                name: "stable-scope".to_owned(),
                enabled: true,
                provider_id: None,
                max_attempts: 2,
                max_duration_seconds: 60,
                max_same_error: 1,
                max_power_cycles: 0,
                max_flash_retries: 0,
                wait_device_seconds: 10,
                cost_limit: None,
                allowed_actions: vec![],
                approval_actions: vec![],
                board_scope: vec!["linux:1".to_owned()],
                success_summary: true,
                notification_mode: "desktop".to_owned(),
            },
        )
        .unwrap();
        create_or_get_run(&supervisor, "run-old", "task-old", "linux:1", policy_id).unwrap();
        insert_system_audit_for_run(
            &supervisor,
            Some("run-old"),
            "linux:1",
            "started",
            "ok",
            &serde_json::json!({}),
            &serde_json::json!({}),
        )
        .unwrap();

        assert_eq!(
            sync_board_identities(&mut supervisor, &workbench).unwrap(),
            1
        );
        let automatic_task_id = save_automatic_task(
            &supervisor,
            &AutomaticTask {
                id: None,
                name: "stable-task".to_owned(),
                enabled: false,
                trigger_type: "manual_api".to_owned(),
                trigger: serde_json::json!({}),
                board_id: "linux:1".to_owned(),
                task_template: serde_json::json!({"kind":"workflow"}),
                policy_id: Some(policy_id),
                success_conditions: vec![serde_json::json!({"kind":"status","equals":"success"})],
                next_run_at: None,
                last_run_at: None,
            },
        )
        .unwrap();
        assert_eq!(
            sync_board_identities(&mut supervisor, &workbench).unwrap(),
            1
        );
        for (table, selector) in [
            ("supervisor_run", "id='run-old'"),
            ("supervisor_audit", "run_id='run-old'"),
        ] {
            let uid: String = supervisor
                .query_row(
                    &format!("SELECT board_uid FROM {table} WHERE {selector}"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(uid, "linux-11111111111111111111111111111111");
        }
        let policy_uids: String = supervisor
            .query_row(
                "SELECT board_uid_scope_json FROM supervisor_policy WHERE id=?1",
                [policy_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(policy_uids, r#"["linux-11111111111111111111111111111111"]"#);
        let task_uid: String = supervisor
            .query_row(
                "SELECT board_uid FROM supervisor_automatic_task WHERE id=?1",
                [automatic_task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(task_uid, "linux-11111111111111111111111111111111");
        assert_eq!(list_runs(&supervisor, 1).unwrap()[0].board_id, "linux:1");
    }

    #[test]
    fn stable_board_identity_sync_rejects_legacy_scope_reassignment() {
        let mut supervisor = Connection::open_in_memory().unwrap();
        migrate(&supervisor).unwrap();
        supervisor
            .execute(
                "INSERT INTO supervisor_board_identity VALUES('linux:1',?1,unixepoch())",
                ["linux-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
            )
            .unwrap();
        let workbench = Connection::open_in_memory().unwrap();
        crate::workbench::storage::migrate(&workbench).unwrap();
        workbench
            .execute(
                "INSERT INTO devices(board_uid,name) VALUES(?1,'conflict')",
                ["linux-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            )
            .unwrap();
        let error = sync_board_identities(&mut supervisor, &workbench)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SUPERVISOR_BOARD_IDENTITY_CONFLICT:linux:1"));
    }

    #[test]
    fn provider_database_has_no_secret_column() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let mut statement = db
            .prepare("SELECT name FROM pragma_table_info('ai_provider_config')")
            .unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| {
            let normalized = column.to_ascii_lowercase();
            normalized.contains("key")
                || normalized.contains("secret")
                || normalized.contains("credential")
        }));
    }

    #[test]
    fn empty_database_queries_are_valid_sql() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        assert!(list_providers(&db).unwrap().is_empty());
        assert!(list_policies(&db).unwrap().is_empty());
        assert!(list_audit(&db, 10).unwrap().is_empty());
        assert!(list_automatic_tasks(&db).unwrap().is_empty());
        assert!(list_runs(&db, 10).unwrap().is_empty());
        assert!(list_rounds(&db, "missing", 10).unwrap().is_empty());
        assert!(list_human_requests(&db, false).unwrap().is_empty());
        assert!(list_human_requests(&db, true).unwrap().is_empty());
        assert_eq!(
            repeated_decision_count(&db, "missing", "fingerprint", "rescan_usb").unwrap(),
            0
        );
        assert!(round_for_execution(&db, "missing", 1).unwrap().is_none());
        assert_eq!(
            finish_round_action(
                &db,
                "missing",
                1,
                &serde_json::json!({}),
                &serde_json::json!({}),
                "rescan_usb"
            )
            .unwrap(),
            0
        );
        assert!(resolve_human_request(&db, 1, "terminate")
            .unwrap()
            .is_none());
        mark_automatic_task_run(&db, 1, 1, None).unwrap();
    }

    #[test]
    fn policy_human_and_automatic_task_round_trip() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let policy_id = save_policy(
            &db,
            &SupervisorPolicy {
                id: None,
                name: "live-safe".to_owned(),
                enabled: true,
                provider_id: None,
                max_attempts: 2,
                max_duration_seconds: 60,
                max_same_error: 1,
                max_power_cycles: 0,
                max_flash_retries: 0,
                wait_device_seconds: 10,
                cost_limit: None,
                allowed_actions: vec!["rescan_usb".to_owned()],
                approval_actions: vec![],
                board_scope: vec!["linux:4".to_owned()],
                success_summary: true,
                notification_mode: "desktop".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(list_policies(&db).unwrap()[0].id, Some(policy_id));
        let mut duplicate = list_policies(&db).unwrap().remove(0);
        duplicate.id = None;
        duplicate.name = "duplicate".to_owned();
        save_policy(&db, &duplicate).unwrap();
        assert_eq!(
            policy_for_board(&db, "linux:4").unwrap_err(),
            "SUPERVISOR_POLICY_BOARD_SCOPE_CONFLICT:linux:4"
        );
        db.execute(
            "UPDATE supervisor_policy SET enabled=0 WHERE name='duplicate'",
            [],
        )
        .unwrap();

        create_or_get_run(&db, "run-1", "task-1", "linux:4", policy_id).unwrap();
        set_run_status(&db, "run-1", "cancelled", "operator cancelled").unwrap();
        let cancelled = list_runs(&db, 10).unwrap().remove(0);
        assert_eq!(cancelled.status, "cancelled");
        assert!(cancelled.finished_at.is_some());
        assert!(set_run_status(&db, "run-1", "running", "retrying")
            .unwrap_err()
            .to_string()
            .contains("SUPERVISOR_STATE_TERMINAL_IMMUTABLE"));
        create_or_get_run(&db, "run-human", "task-human", "linux:4", policy_id).unwrap();
        create_human_request(
            &db,
            "run-human",
            "budget exhausted",
            "inspect hardware",
            &["evidence".to_owned()],
        )
        .unwrap();
        let request = list_human_requests(&db, true).unwrap().remove(0);
        assert_eq!(
            resolve_human_request(&db, request.id, "continue").unwrap(),
            Some("run-human".to_owned())
        );

        let automatic_id = save_automatic_task(
            &db,
            &AutomaticTask {
                id: None,
                name: "manual-safe".to_owned(),
                enabled: false,
                trigger_type: "manual_api".to_owned(),
                trigger: serde_json::json!({}),
                board_id: "linux:4".to_owned(),
                task_template: serde_json::json!({"kind":"workflow","project_id":2,"device_id":4}),
                policy_id: Some(policy_id),
                success_conditions: vec![serde_json::json!({"kind":"status","equals":"success"})],
                next_run_at: None,
                last_run_at: None,
            },
        )
        .unwrap();
        mark_automatic_task_run(&db, automatic_id, 42, None).unwrap();
        assert_eq!(
            automatic_task(&db, automatic_id)
                .unwrap()
                .unwrap()
                .last_run_at,
            Some(42)
        );
    }

    #[test]
    fn startup_reconciles_terminal_workbench_tasks_without_touching_human_waits() {
        let supervisor = Connection::open_in_memory().unwrap();
        migrate(&supervisor).unwrap();
        let policy_id = save_policy(
            &supervisor,
            &SupervisorPolicy {
                id: None,
                name: "reconcile".to_owned(),
                enabled: true,
                provider_id: None,
                max_attempts: 1,
                max_duration_seconds: 60,
                max_same_error: 1,
                max_power_cycles: 0,
                max_flash_retries: 0,
                wait_device_seconds: 10,
                cost_limit: None,
                allowed_actions: vec!["rescan_usb".to_owned()],
                approval_actions: vec![],
                board_scope: vec!["linux:4".to_owned()],
                success_summary: true,
                notification_mode: "desktop".to_owned(),
            },
        )
        .unwrap();
        create_or_get_run(
            &supervisor,
            "run-failed",
            "task-failed",
            "linux:4",
            policy_id,
        )
        .unwrap();
        create_or_get_run(&supervisor, "run-human", "task-human", "linux:4", policy_id).unwrap();
        set_run_status(&supervisor, "run-human", "waiting_human", "inspect").unwrap();

        let workbench = Connection::open_in_memory().unwrap();
        workbench
            .execute_batch(
                "CREATE TABLE task_history(task_id TEXT PRIMARY KEY,status TEXT NOT NULL,error TEXT NOT NULL);\
                 INSERT INTO task_history VALUES('task-failed','failed','SERIAL_TIMEOUT');\
                 INSERT INTO task_history VALUES('task-human','failed','NEEDS_OPERATOR');",
            )
            .unwrap();

        assert_eq!(
            reconcile_interrupted_runs(&supervisor, &workbench).unwrap(),
            1
        );
        let runs = list_runs(&supervisor, 10).unwrap();
        let failed = runs.iter().find(|run| run.id == "run-failed").unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.final_reason, "APP_INTERRUPTED:SERIAL_TIMEOUT");
        assert_eq!(
            runs.iter()
                .find(|run| run.id == "run-human")
                .unwrap()
                .status,
            "waiting_human"
        );
    }

    #[test]
    #[ignore = "requires the explicitly configured live supervisor database and keyring"]
    fn live_database_views_load_without_secrets() {
        let database = std::env::var("LYNX_LIVE_SUPERVISOR_DB")
            .expect("LYNX_LIVE_SUPERVISOR_DB must point to the live test database");
        let db = Connection::open(database).expect("live supervisor database must open");
        migrate(&db).unwrap();
        let providers = list_providers(&db).unwrap();
        assert!(providers.iter().any(|provider| provider.has_api_key));
        assert!(!list_policies(&db).unwrap().is_empty());
        assert!(!list_audit(&db, 100).unwrap().is_empty());
        assert!(!list_automatic_tasks(&db).unwrap().is_empty());
        assert!(!list_runs(&db, 100).unwrap().is_empty());
        assert!(!list_human_requests(&db, false).unwrap().is_empty());
    }
}

pub fn list_providers(db: &Connection) -> rusqlite::Result<Vec<AiProviderConfig>> {
    let mut statement = db.prepare(
        "SELECT id,name,preset,base_url,model,timeout_seconds,max_tokens,task_cost_limit, \
         input_cost_per_million,output_cost_per_million,enabled,is_default,last_test_status,last_test_message,last_test_at \
         FROM ai_provider_config ORDER BY is_default DESC,name,id",
    )?;
    let rows = statement
        .query_map([], |row| {
            let id = row.get::<_, i64>(0)?;
            Ok(AiProviderConfig {
                id: Some(id),
                name: row.get(1)?,
                preset: row.get(2)?,
                base_url: row.get(3)?,
                model: row.get(4)?,
                timeout_seconds: row.get(5)?,
                max_tokens: row.get(6)?,
                task_cost_limit: row.get(7)?,
                input_cost_per_million: row.get(8)?,
                output_cost_per_million: row.get(9)?,
                enabled: row.get(10)?,
                is_default: row.get(11)?,
                has_api_key: credentials::exists(id),
                last_test_status: row.get(12)?,
                last_test_message: row.get(13)?,
                last_test_at: row.get(14)?,
            })
        })?
        .collect();
    rows
}

pub fn provider(db: &Connection, id: i64) -> rusqlite::Result<Option<AiProviderConfig>> {
    Ok(list_providers(db)?
        .into_iter()
        .find(|provider| provider.id == Some(id)))
}

pub fn save_provider(db: &Connection, request: &SaveAiProviderRequest) -> rusqlite::Result<i64> {
    if request.is_default {
        db.execute("UPDATE ai_provider_config SET is_default=0", [])?;
    }
    if let Some(id) = request.id {
        let changed = db.execute(
            "UPDATE ai_provider_config SET name=?1,preset=?2,base_url=?3,model=?4,\
             timeout_seconds=?5,max_tokens=?6,task_cost_limit=?7,input_cost_per_million=?8,\
             output_cost_per_million=?9,enabled=?10,is_default=?11 WHERE id=?12",
            params![
                request.name,
                request.preset,
                request.base_url,
                request.model,
                request.timeout_seconds,
                request.max_tokens,
                request.task_cost_limit,
                request.input_cost_per_million,
                request.output_cost_per_million,
                request.enabled,
                request.is_default,
                id
            ],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        return Ok(id);
    }
    db.execute(
        "INSERT INTO ai_provider_config(name,preset,base_url,model,timeout_seconds,max_tokens,\
         task_cost_limit,input_cost_per_million,output_cost_per_million,enabled,is_default)\
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            request.name,
            request.preset,
            request.base_url,
            request.model,
            request.timeout_seconds,
            request.max_tokens,
            request.task_cost_limit,
            request.input_cost_per_million,
            request.output_cost_per_million,
            request.enabled,
            request.is_default
        ],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn update_test_result(
    db: &Connection,
    id: i64,
    status: &str,
    message: &str,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE ai_provider_config SET last_test_status=?1,last_test_message=?2, \
         last_test_at=unixepoch() WHERE id=?3",
        params![status, message, id],
    )?;
    Ok(())
}

pub fn delete_provider(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM ai_provider_config WHERE id=?1", [id])?;
    Ok(())
}

pub fn default_provider_id(db: &Connection) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT id FROM ai_provider_config WHERE enabled=1 ORDER BY is_default DESC,id LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
}

pub fn list_policies(db: &Connection) -> rusqlite::Result<Vec<SupervisorPolicy>> {
    let mut statement = db.prepare(
        "SELECT id,name,enabled,provider_id,max_attempts,max_duration_seconds,max_same_error, \
         max_power_cycles,max_flash_retries,wait_device_seconds,cost_limit,allowed_actions_json, \
         approval_actions_json,board_scope_json,success_summary,notification_mode \
         FROM supervisor_policy ORDER BY name,id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(SupervisorPolicy {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
                provider_id: row.get(3)?,
                max_attempts: row.get(4)?,
                max_duration_seconds: row.get(5)?,
                max_same_error: row.get(6)?,
                max_power_cycles: row.get(7)?,
                max_flash_retries: row.get(8)?,
                wait_device_seconds: row.get(9)?,
                cost_limit: row.get(10)?,
                allowed_actions: serde_json::from_str(&row.get::<_, String>(11)?)
                    .unwrap_or_default(),
                approval_actions: serde_json::from_str(&row.get::<_, String>(12)?)
                    .unwrap_or_default(),
                board_scope: serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_default(),
                success_summary: row.get(14)?,
                notification_mode: row.get(15)?,
            })
        })?
        .collect();
    rows
}

pub fn save_policy(db: &Connection, policy: &SupervisorPolicy) -> rusqlite::Result<i64> {
    let allowed =
        serde_json::to_string(&policy.allowed_actions).unwrap_or_else(|_| "[]".to_owned());
    let approvals =
        serde_json::to_string(&policy.approval_actions).unwrap_or_else(|_| "[]".to_owned());
    let boards = serde_json::to_string(&policy.board_scope).unwrap_or_else(|_| "[]".to_owned());
    let board_uids = serde_json::to_string(&board_uids_for_scopes(db, &policy.board_scope)?)
        .unwrap_or_else(|_| "[]".to_owned());
    if let Some(id) = policy.id {
        db.execute(
            "UPDATE supervisor_policy SET name=?1,enabled=?2,provider_id=?3,max_attempts=?4,\
             max_duration_seconds=?5,max_same_error=?6,max_power_cycles=?7,max_flash_retries=?8,\
             wait_device_seconds=?9,cost_limit=?10,allowed_actions_json=?11,\
             approval_actions_json=?12,board_scope_json=?13,board_uid_scope_json=?14,\
             success_summary=?15,notification_mode=?16 WHERE id=?17",
            params![
                policy.name,
                policy.enabled,
                policy.provider_id,
                policy.max_attempts,
                policy.max_duration_seconds,
                policy.max_same_error,
                policy.max_power_cycles,
                policy.max_flash_retries,
                policy.wait_device_seconds,
                policy.cost_limit,
                allowed,
                approvals,
                boards,
                board_uids,
                policy.success_summary,
                policy.notification_mode,
                id
            ],
        )?;
        return Ok(id);
    }
    db.execute(
        "INSERT INTO supervisor_policy(name,enabled,provider_id,max_attempts,max_duration_seconds,\
         max_same_error,max_power_cycles,max_flash_retries,wait_device_seconds,cost_limit,\
         allowed_actions_json,approval_actions_json,board_scope_json,board_uid_scope_json,\
         success_summary,notification_mode)\
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            policy.name,
            policy.enabled,
            policy.provider_id,
            policy.max_attempts,
            policy.max_duration_seconds,
            policy.max_same_error,
            policy.max_power_cycles,
            policy.max_flash_retries,
            policy.wait_device_seconds,
            policy.cost_limit,
            allowed,
            approvals,
            boards,
            board_uids,
            policy.success_summary,
            policy.notification_mode
        ],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn delete_policy(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM supervisor_policy WHERE id=?1", [id])?;
    Ok(())
}

pub fn overview(db: &Connection) -> rusqlite::Result<SupervisorOverview> {
    Ok(SupervisorOverview {
        provider_count: db.query_row("SELECT count(*) FROM ai_provider_config", [], |row| row.get(0))?,
        ready_provider_count: list_providers(db)?
            .into_iter()
            .filter(|provider| provider.enabled && provider.has_api_key)
            .count() as u64,
        enabled_policy_count: db.query_row(
            "SELECT count(*) FROM supervisor_policy WHERE enabled=1",
            [],
            |row| row.get(0),
        )?,
        active_run_count: db.query_row(
            "SELECT count(*) FROM supervisor_run WHERE status IN ('running','paused','waiting_approval')",
            [],
            |row| row.get(0),
        )?,
        waiting_human_count: db.query_row(
            "SELECT count(*) FROM supervisor_human_request WHERE status='waiting'",
            [],
            |row| row.get(0),
        )?,
        today_call_count: db.query_row(
            "SELECT count(*) FROM supervisor_audit WHERE event_type='ai_call' AND created_at >= unixepoch('now','start of day')",
            [],
            |row| row.get(0),
        )?,
        today_estimated_cost: db.query_row(
            "SELECT COALESCE(sum(estimated_cost),0) FROM supervisor_audit WHERE event_type='ai_call' AND created_at >= unixepoch('now','start of day')",
            [],
            |row| row.get(0),
        )?,
    })
}

pub fn list_audit(db: &Connection, limit: u32) -> rusqlite::Result<Vec<SupervisorAuditRecord>> {
    let mut statement = db.prepare(
        "SELECT id,run_id,board_id,provider_id,event_type,result,redacted_input_json,output_json, \
         input_tokens,output_tokens,estimated_cost,created_at FROM supervisor_audit \
         ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit], |row| {
            Ok(SupervisorAuditRecord {
                id: row.get(0)?,
                run_id: row.get(1)?,
                board_id: row.get(2)?,
                provider_id: row.get(3)?,
                event_type: row.get(4)?,
                result: row.get(5)?,
                redacted_input: serde_json::from_str(&row.get::<_, String>(6)?)
                    .unwrap_or_else(|_| serde_json::json!({"redacted": true})),
                output: serde_json::from_str(&row.get::<_, String>(7)?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                input_tokens: row.get(8)?,
                output_tokens: row.get(9)?,
                estimated_cost: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .collect();
    rows
}

pub fn list_automatic_tasks(db: &Connection) -> rusqlite::Result<Vec<AutomaticTask>> {
    let mut statement = db.prepare(
        "SELECT id,name,enabled,trigger_type,trigger_json,board_id,task_template_json,policy_id,\
         success_conditions_json,next_run_at,last_run_at FROM supervisor_automatic_task ORDER BY name,id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(AutomaticTask {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
                trigger_type: row.get(3)?,
                trigger: serde_json::from_str(&row.get::<_, String>(4)?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                board_id: row.get(5)?,
                task_template: serde_json::from_str(&row.get::<_, String>(6)?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                policy_id: row.get(7)?,
                success_conditions: serde_json::from_str(&row.get::<_, String>(8)?)
                    .unwrap_or_default(),
                next_run_at: row.get(9)?,
                last_run_at: row.get(10)?,
            })
        })?
        .collect();
    rows
}

pub fn save_automatic_task(db: &Connection, task: &AutomaticTask) -> rusqlite::Result<i64> {
    let trigger = serde_json::to_string(&task.trigger).unwrap_or_else(|_| "{}".to_owned());
    let template = serde_json::to_string(&task.task_template).unwrap_or_else(|_| "{}".to_owned());
    let success =
        serde_json::to_string(&task.success_conditions).unwrap_or_else(|_| "[]".to_owned());
    let board_uid = board_uid_for_scope(db, &task.board_id)?;
    if let Some(id) = task.id {
        db.execute(
            "UPDATE supervisor_automatic_task SET name=?1,enabled=?2,trigger_type=?3,trigger_json=?4,\
             board_id=?5,board_uid=?6,task_template_json=?7,policy_id=?8,\
             success_conditions_json=?9,next_run_at=?10,updated_at=unixepoch() WHERE id=?11",
            params![task.name, task.enabled, task.trigger_type, trigger, task.board_id, board_uid,
                template, task.policy_id, success, task.next_run_at, id],
        )?;
        return Ok(id);
    }
    db.execute(
        "INSERT INTO supervisor_automatic_task(name,enabled,trigger_type,trigger_json,board_id,\
         board_uid,task_template_json,policy_id,success_conditions_json,next_run_at,created_at,updated_at)\
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,unixepoch(),unixepoch())",
        params![
            task.name,
            task.enabled,
            task.trigger_type,
            trigger,
            task.board_id,
            board_uid,
            template,
            task.policy_id,
            success,
            task.next_run_at
        ],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn delete_automatic_task(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM supervisor_automatic_task WHERE id=?1", [id])?;
    Ok(())
}

pub fn automatic_task(db: &Connection, id: i64) -> rusqlite::Result<Option<AutomaticTask>> {
    Ok(list_automatic_tasks(db)?
        .into_iter()
        .find(|task| task.id == Some(id)))
}

pub fn due_automatic_tasks(db: &Connection, now: i64) -> rusqlite::Result<Vec<AutomaticTask>> {
    Ok(list_automatic_tasks(db)?
        .into_iter()
        .filter(|task| {
            task.enabled
                && task.trigger_type == "schedule"
                && task.next_run_at.is_some_and(|next| next <= now)
        })
        .collect())
}

pub fn mark_automatic_task_run(
    db: &Connection,
    id: i64,
    last_run_at: i64,
    next_run_at: Option<i64>,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE supervisor_automatic_task SET last_run_at=?1,next_run_at=?2,updated_at=unixepoch() \
         WHERE id=?3",
        params![last_run_at, next_run_at, id],
    )?;
    Ok(())
}

pub fn insert_system_audit_for_run(
    db: &Connection,
    run_id: Option<&str>,
    board_id: &str,
    event_type: &str,
    result: &str,
    input: &Value,
    output: &Value,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO supervisor_audit(run_id,board_id,board_uid,event_type,result,redacted_input_json,output_json,created_at)\
         VALUES(?1,?2,?3,?4,?5,?6,?7,unixepoch())",
        params![run_id, board_id, board_uid_for_scope(db, board_id)?, event_type, result,
            input.to_string(), output.to_string()],
    )?;
    Ok(())
}

pub fn policy(db: &Connection, id: i64) -> rusqlite::Result<Option<SupervisorPolicy>> {
    Ok(list_policies(db)?
        .into_iter()
        .find(|policy| policy.id == Some(id)))
}

pub fn policy_for_board(
    db: &Connection,
    board_id: &str,
) -> Result<Option<SupervisorPolicy>, String> {
    let matches = list_policies(db)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|policy| {
            policy.enabled
                && policy
                    .board_scope
                    .iter()
                    .any(|candidate| candidate == board_id)
        })
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(format!("SUPERVISOR_POLICY_BOARD_SCOPE_CONFLICT:{board_id}")),
    }
}

pub fn create_or_get_run(
    db: &Connection,
    run_id: &str,
    task_id: &str,
    board_id: &str,
    policy_id: i64,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO supervisor_run(id,task_id,board_id,board_uid,policy_id,status,started_at)\
         VALUES(?1,?2,?3,?4,?5,'running',unixepoch())",
        params![run_id, task_id, board_id, board_uid_for_scope(db, board_id)?, policy_id],
    )?;
    Ok(())
}

pub fn run_scope(db: &Connection, run_id: &str) -> rusqlite::Result<Option<(String, String, i64)>> {
    db.query_row(
        "SELECT task_id,board_id,policy_id FROM supervisor_run WHERE id=?1",
        [run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
}

pub struct RunBudget {
    pub status: String,
    pub started_at: i64,
    pub power_cycle_count: u32,
    pub flash_retry_count: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub estimated_cost: f64,
}

pub fn run_budget(db: &Connection, run_id: &str) -> rusqlite::Result<Option<RunBudget>> {
    db.query_row(
        "SELECT status,started_at,power_cycle_count,flash_retry_count,total_input_tokens,\
         total_output_tokens,estimated_cost FROM supervisor_run WHERE id=?1",
        [run_id],
        |row| {
            Ok(RunBudget {
                status: row.get(0)?,
                started_at: row.get(1)?,
                power_cycle_count: row.get(2)?,
                flash_retry_count: row.get(3)?,
                total_input_tokens: row.get(4)?,
                total_output_tokens: row.get(5)?,
                estimated_cost: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn round_count(db: &Connection, run_id: &str) -> rusqlite::Result<u32> {
    db.query_row(
        "SELECT count(*) FROM supervisor_round WHERE run_id=?1",
        [run_id],
        |row| row.get(0),
    )
}

pub fn repeated_decision_count(
    db: &Connection,
    run_id: &str,
    fingerprint: &str,
    decision: &str,
) -> rusqlite::Result<u32> {
    db.query_row(
        "SELECT count(*) FROM supervisor_round WHERE run_id=?1 AND failure_fingerprint=?2 \
         AND json_extract(decision_json,'$.decision')=?3",
        params![run_id, fingerprint, decision],
        |row| row.get(0),
    )
}

pub struct RoundRecord<'a> {
    pub run_id: &'a str,
    pub round_index: u32,
    pub fingerprint: &'a str,
    pub context: &'a serde_json::Value,
    pub decision: &'a SupervisorDecision,
    pub schema_result: &'a str,
    pub policy_result: &'a str,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
}

pub fn insert_round(db: &Connection, record: RoundRecord<'_>) -> rusqlite::Result<()> {
    let current_status: String = db.query_row(
        "SELECT status FROM supervisor_run WHERE id=?1",
        [record.run_id],
        |row| row.get(0),
    )?;
    let next_status = if record.policy_result == "approval_required" {
        "waiting_approval"
    } else {
        "running"
    };
    crate::task_lifecycle::validate_supervisor_transition(&current_status, next_status)
        .map_err(|code| rusqlite::Error::InvalidParameterName(code.to_owned()))?;
    db.execute(
        "INSERT INTO supervisor_round(run_id,round_index,failure_fingerprint,redacted_context_json,\
         decision_json,schema_result,policy_result,action_result,verification_result,input_tokens,\
         output_tokens,estimated_cost,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'not_executed','pending',?8,?9,?10,unixepoch())",
        params![record.run_id, record.round_index, record.fingerprint,
            record.context.to_string(), serde_json::to_string(record.decision).unwrap_or_default(),
            record.schema_result, record.policy_result, record.input_tokens, record.output_tokens,
            record.estimated_cost],
    )?;
    db.execute(
        "UPDATE supervisor_run SET attempt_count=?1,total_input_tokens=total_input_tokens+?2,\
         total_output_tokens=total_output_tokens+?3,estimated_cost=estimated_cost+?4,status=?5 WHERE id=?6",
        params![
            record.round_index,
            record.input_tokens,
            record.output_tokens,
            record.estimated_cost,
            next_status,
            record.run_id
        ],
    )?;
    Ok(())
}

pub struct AuditInsert<'a> {
    pub run_id: &'a str,
    pub board_id: &'a str,
    pub provider_id: i64,
    pub result: &'a str,
    pub context: &'a serde_json::Value,
    pub decision: &'a SupervisorDecision,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
}

pub fn insert_audit(db: &Connection, record: AuditInsert<'_>) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO supervisor_audit(run_id,board_id,board_uid,provider_id,event_type,result,\
         redacted_input_json,output_json,input_tokens,output_tokens,estimated_cost,created_at)\
         VALUES(?1,?2,?3,?4,'ai_call',?5,?6,?7,?8,?9,?10,unixepoch())",
        params![
            record.run_id,
            record.board_id,
            board_uid_for_scope(db, record.board_id)?,
            record.provider_id,
            record.result,
            record.context.to_string(),
            serde_json::to_string(record.decision).unwrap_or_default(),
            record.input_tokens,
            record.output_tokens,
            record.estimated_cost
        ],
    )?;
    Ok(())
}

pub fn list_runs(db: &Connection, limit: u32) -> rusqlite::Result<Vec<SupervisorRun>> {
    let mut statement = db.prepare(
        "SELECT id,task_id,board_id,policy_id,status,attempt_count,power_cycle_count,\
         flash_retry_count,started_at,finished_at,total_input_tokens,total_output_tokens,\
         estimated_cost,final_reason FROM supervisor_run ORDER BY started_at DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit], |row| {
            Ok(SupervisorRun {
                id: row.get(0)?,
                task_id: row.get(1)?,
                board_id: row.get(2)?,
                policy_id: row.get(3)?,
                status: row.get(4)?,
                attempt_count: row.get(5)?,
                power_cycle_count: row.get(6)?,
                flash_retry_count: row.get(7)?,
                started_at: row.get(8)?,
                finished_at: row.get(9)?,
                total_input_tokens: row.get(10)?,
                total_output_tokens: row.get(11)?,
                estimated_cost: row.get(12)?,
                final_reason: row.get(13)?,
            })
        })?
        .collect();
    rows
}

pub fn list_rounds(
    db: &Connection,
    run_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<SupervisorRound>> {
    let mut statement = db.prepare(
        "SELECT id,run_id,round_index,failure_fingerprint,redacted_context_json,decision_json,\
         schema_result,policy_result,action_result,verification_result,input_tokens,output_tokens,\
         estimated_cost,created_at FROM supervisor_round WHERE run_id=?1 \
         ORDER BY round_index DESC LIMIT ?2",
    )?;
    let rows = statement
        .query_map(params![run_id, limit], |row| {
            let decision_json: String = row.get(5)?;
            let decision = serde_json::from_str(&decision_json).unwrap_or(SupervisorDecision {
                decision: "finish_failed".to_owned(),
                reason: "Stored decision could not be decoded".to_owned(),
                confidence: 0.0,
                evidence: vec!["SUPERVISOR_DECISION_DECODE_FAILED".to_owned()],
                parameters: serde_json::json!({}),
                expected_result: String::new(),
                need_human: true,
                human_instruction: Some("查看原始审计记录".to_owned()),
            });
            Ok(SupervisorRound {
                id: row.get(0)?,
                run_id: row.get(1)?,
                round_index: row.get(2)?,
                failure_fingerprint: row.get(3)?,
                redacted_context: serde_json::from_str(&row.get::<_, String>(4)?)
                    .unwrap_or_else(|_| serde_json::json!({"redacted": true})),
                decision,
                schema_result: row.get(6)?,
                policy_result: row.get(7)?,
                action_result: row.get(8)?,
                verification_result: row.get(9)?,
                input_tokens: row.get(10)?,
                output_tokens: row.get(11)?,
                estimated_cost: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?
        .collect();
    rows
}

pub fn set_run_status(
    db: &Connection,
    run_id: &str,
    status: &str,
    reason: &str,
) -> rusqlite::Result<usize> {
    let current = db
        .query_row(
            "SELECT status FROM supervisor_run WHERE id=?1",
            [run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(current) = current else {
        return Ok(0);
    };
    crate::task_lifecycle::validate_supervisor_transition(&current, status)
        .map_err(|code| rusqlite::Error::InvalidParameterName(code.to_owned()))?;
    if current == status {
        return Ok(1);
    }
    let terminal = crate::task_lifecycle::SupervisorRunState::parse(status)
        .is_some_and(crate::task_lifecycle::SupervisorRunState::is_terminal);
    db.execute(
        "UPDATE supervisor_run SET status=?1,final_reason=?2,\
         finished_at=CASE WHEN ?3 THEN unixepoch() ELSE NULL END WHERE id=?4 AND status=?5",
        params![status, reason, terminal, run_id, current],
    )
}

pub fn reconcile_interrupted_runs(
    supervisor: &Connection,
    workbench: &Connection,
) -> rusqlite::Result<usize> {
    let mut statement = supervisor.prepare(
        "SELECT id,task_id FROM supervisor_run \
         WHERE status IN ('running','paused','waiting_approval')",
    )?;
    let active_runs = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reconciled = 0;
    for (run_id, task_id) in active_runs {
        let task = workbench
            .query_row(
                "SELECT status,error FROM task_history WHERE task_id=?1",
                [&task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((task_status, task_error)) = task else {
            continue;
        };
        let (status, reason) = match task_status.as_str() {
            "success" => ("success", "底层任务已通过确定性验收".to_owned()),
            "cancelled" => ("cancelled", "底层任务已取消".to_owned()),
            "failed" => (
                "failed",
                if task_error.is_empty() {
                    "APP_INTERRUPTED:应用重启前底层任务已失败，原监督恢复进程已结束".to_owned()
                } else {
                    format!("APP_INTERRUPTED:{task_error}")
                },
            ),
            _ => continue,
        };
        reconciled += set_run_status(supervisor, &run_id, status, &reason)?;
    }
    Ok(reconciled)
}

pub fn create_human_request(
    db: &Connection,
    run_id: &str,
    reason: &str,
    instruction: &str,
    evidence: &[String],
) -> rusqlite::Result<()> {
    let existing: u32 = db.query_row(
        "SELECT count(*) FROM supervisor_human_request WHERE run_id=?1 AND status='waiting'",
        [run_id],
        |row| row.get(0),
    )?;
    if existing == 0 {
        db.execute(
            "INSERT INTO supervisor_human_request(run_id,reason,instruction,evidence_json,created_at)\
             VALUES(?1,?2,?3,?4,unixepoch())",
            params![run_id, reason, instruction,
                serde_json::to_string(evidence).unwrap_or_else(|_| "[]".to_owned())],
        )?;
    }
    set_run_status(db, run_id, "waiting_human", reason)?;
    Ok(())
}

pub fn list_human_requests(
    db: &Connection,
    waiting_only: bool,
) -> rusqlite::Result<Vec<HumanInterventionRequest>> {
    let sql = if waiting_only {
        "SELECT id,run_id,status,reason,instruction,evidence_json,resolution,created_at,resolved_at \
         FROM supervisor_human_request WHERE status='waiting' ORDER BY id DESC"
    } else {
        "SELECT id,run_id,status,reason,instruction,evidence_json,resolution,created_at,resolved_at \
         FROM supervisor_human_request ORDER BY id DESC LIMIT 200"
    };
    let mut statement = db.prepare(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok(HumanInterventionRequest {
                id: row.get(0)?,
                run_id: row.get(1)?,
                status: row.get(2)?,
                reason: row.get(3)?,
                instruction: row.get(4)?,
                evidence: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                resolution: row.get(6)?,
                created_at: row.get(7)?,
                resolved_at: row.get(8)?,
            })
        })?
        .collect();
    rows
}

pub fn resolve_human_request(
    db: &Connection,
    request_id: i64,
    resolution: &str,
) -> rusqlite::Result<Option<String>> {
    let run_id = db
        .query_row(
            "SELECT run_id FROM supervisor_human_request WHERE id=?1 AND status='waiting'",
            [request_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    db.execute(
        "UPDATE supervisor_human_request SET status='resolved',resolution=?1,resolved_at=unixepoch() \
         WHERE id=?2",
        params![resolution, request_id],
    )?;
    Ok(Some(run_id))
}

pub fn round_for_execution(
    db: &Connection,
    run_id: &str,
    round_index: u32,
) -> rusqlite::Result<Option<(SupervisorDecision, String, String)>> {
    db.query_row(
        "SELECT decision_json,policy_result,action_result FROM supervisor_round \
         WHERE run_id=?1 AND round_index=?2",
        params![run_id, round_index],
        |row| {
            let decision_json = row.get::<_, String>(0)?;
            let decision = serde_json::from_str(&decision_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    decision_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok((decision, row.get(1)?, row.get(2)?))
        },
    )
    .optional()
}

pub fn finish_round_action(
    db: &Connection,
    run_id: &str,
    round_index: u32,
    action_result: &serde_json::Value,
    verification_result: &serde_json::Value,
    action: &str,
) -> rusqlite::Result<usize> {
    let changed = db.execute(
        "UPDATE supervisor_round SET action_result=?1,verification_result=?2 \
         WHERE run_id=?3 AND round_index=?4 AND action_result='not_executed'",
        params![
            action_result.to_string(),
            verification_result.to_string(),
            run_id,
            round_index
        ],
    )?;
    if changed > 0 {
        db.execute(
            "UPDATE supervisor_run SET \
             power_cycle_count=power_cycle_count+CASE WHEN ?1='power_cycle' THEN 1 ELSE 0 END, \
             flash_retry_count=flash_retry_count+CASE WHEN ?1='retry_flash' THEN 1 ELSE 0 END \
             WHERE id=?2",
            params![action, run_id],
        )?;
    }
    Ok(changed)
}

pub fn fail_round_action(
    db: &Connection,
    run_id: &str,
    round_index: u32,
    action: &str,
    error: &str,
) -> rusqlite::Result<usize> {
    finish_round_action(
        db,
        run_id,
        round_index,
        &serde_json::json!({"status": "failed", "error": error}),
        &serde_json::json!({"kind": "action_failed", "verified": false}),
        action,
    )
}

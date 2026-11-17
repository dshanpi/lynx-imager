use rusqlite::{params, Connection, OptionalExtension};

use super::types::{
    BackupRecord, FeedbackDraft, RecordEvent, RecordOverview, RecordSession,
    SaveFeedbackDraftRequest,
};

const MIGRATIONS: &[crate::database_migrations::Migration] = &[
    crate::database_migrations::Migration {
        version: 1,
        name: "initial_record_center_schema",
        apply: migrate_initial_schema,
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
    db.execute_batch("PRAGMA foreign_keys = ON;")?;
    crate::database_migrations::apply(db, "records", MIGRATIONS)
}

fn migrate_initial_schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS record_event (
            event_id TEXT PRIMARY KEY,
            occurred_at_ms INTEGER NOT NULL,
            actor_type TEXT NOT NULL,
            actor_id TEXT NOT NULL,
            category TEXT NOT NULL,
            action TEXT NOT NULL,
            scope_type TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            session_id TEXT,
            correlation_id TEXT NOT NULL,
            causation_id TEXT,
            result TEXT NOT NULL,
            error_code TEXT,
            summary TEXT NOT NULL,
            metadata_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_record_event_time ON record_event(occurred_at_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_record_event_session ON record_event(session_id, occurred_at_ms);
        CREATE INDEX IF NOT EXISTS idx_record_event_scope ON record_event(scope_type, scope_id, occurred_at_ms DESC);
        CREATE TABLE IF NOT EXISTS record_session (
            session_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            scope_type TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL,
            ended_at_ms INTEGER,
            integrity TEXT NOT NULL DEFAULT 'complete'
        );
        CREATE TABLE IF NOT EXISTS feedback_draft (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            problem_type TEXT NOT NULL,
            page TEXT NOT NULL,
            expected TEXT NOT NULL,
            actual TEXT NOT NULL,
            reproducible TEXT NOT NULL,
            status TEXT NOT NULL,
            event_ids_json TEXT NOT NULL,
            include_sensitive_body INTEGER NOT NULL DEFAULT 0,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS record_backup (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            sha256 TEXT NOT NULL,
            event_count INTEGER NOT NULL,
            session_count INTEGER NOT NULL,
            feedback_count INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        "#,
    )
}

fn migrate_stable_board_identity_mirror(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS record_board_identity (
            legacy_scope_id TEXT PRIMARY KEY,
            board_uid TEXT NOT NULL UNIQUE,
            synced_at INTEGER NOT NULL
        );",
    )?;
    add_column(db, "record_event", "board_uid TEXT NOT NULL DEFAULT ''")?;
    add_column(db, "record_session", "board_uid TEXT NOT NULL DEFAULT ''")?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_record_event_board_uid ON record_event(board_uid,occurred_at_ms DESC)",
        [],
    )?;
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
    records: &mut Connection,
    workbench: &Connection,
) -> rusqlite::Result<usize> {
    let devices = crate::workbench::storage::list_devices(workbench)?;
    let transaction = records.transaction()?;
    let mut synced = 0;
    for device in devices {
        let (Some(id), board_uid) = (device.id, device.board_uid.trim()) else {
            continue;
        };
        if board_uid.is_empty() {
            continue;
        }
        let scope_id = format!("linux:{id}");
        let historical_conflict: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM record_event WHERE scope_id=?1 AND board_uid<>'' AND board_uid<>?2
                UNION ALL
                SELECT 1 FROM record_session WHERE scope_id=?1 AND board_uid<>'' AND board_uid<>?2
            )",
            params![scope_id, board_uid],
            |row| row.get(0),
        )?;
        if historical_conflict {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "RECORD_BOARD_IDENTITY_CONFLICT:{scope_id}"
            )));
        }
        transaction.execute(
            "INSERT INTO record_board_identity(legacy_scope_id,board_uid,synced_at)
             VALUES(?1,?2,unixepoch()) ON CONFLICT(legacy_scope_id) DO NOTHING",
            params![scope_id, board_uid],
        )?;
        let stored_uid: String = transaction.query_row(
            "SELECT board_uid FROM record_board_identity WHERE legacy_scope_id=?1",
            [&scope_id],
            |row| row.get(0),
        )?;
        if stored_uid != board_uid {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "RECORD_BOARD_IDENTITY_CONFLICT:{scope_id}"
            )));
        }
        transaction.execute(
            "UPDATE record_board_identity SET synced_at=unixepoch() WHERE legacy_scope_id=?1",
            [&scope_id],
        )?;
        synced += 1;
    }
    transaction.execute(
        "UPDATE record_event SET board_uid=COALESCE((
            SELECT board_uid FROM record_board_identity WHERE legacy_scope_id=scope_id
         ),board_uid) WHERE board_uid=''",
        [],
    )?;
    transaction.execute(
        "UPDATE record_session SET board_uid=COALESCE((
            SELECT board_uid FROM record_board_identity WHERE legacy_scope_id=scope_id
         ),board_uid) WHERE board_uid=''",
        [],
    )?;
    transaction.commit()?;
    Ok(synced)
}

pub fn board_uid_for_scope(db: &Connection, scope_id: &str) -> rusqlite::Result<String> {
    Ok(db
        .query_row(
            "SELECT board_uid FROM record_board_identity WHERE legacy_scope_id=?1",
            [scope_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

pub fn validate_restored_board_identities(
    db: &Connection,
    events: &[RecordEvent],
    sessions: &[RecordSession],
) -> rusqlite::Result<()> {
    for (scope_id, board_uid) in events
        .iter()
        .map(|item| (&item.scope_id, &item.board_uid))
        .chain(
            sessions
                .iter()
                .map(|item| (&item.scope_id, &item.board_uid)),
        )
    {
        if board_uid.is_empty() || !scope_id.starts_with("linux:") {
            continue;
        }
        let known = board_uid_for_scope(db, scope_id)?;
        if !known.is_empty() && known != *board_uid {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "RESTORE_BOARD_IDENTITY_CONFLICT:{scope_id}"
            )));
        }
    }
    Ok(())
}

pub fn insert_event(db: &Connection, event: &RecordEvent) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO record_event(event_id,occurred_at_ms,actor_type,actor_id,category,action,
         scope_type,scope_id,session_id,correlation_id,causation_id,result,error_code,summary,
         metadata_json,board_uid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            event.event_id,
            event.occurred_at_ms,
            event.actor_type,
            event.actor_id,
            event.category,
            event.action,
            event.scope_type,
            event.scope_id,
            event.session_id,
            event.correlation_id,
            event.causation_id,
            event.result,
            event.error_code,
            event.summary,
            event.metadata.to_string(),
            event.board_uid,
        ],
    )?;
    Ok(())
}

pub fn end_session(db: &Connection, session_id: &str, ended_at_ms: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE record_session SET ended_at_ms=?2 WHERE session_id=?1 AND ended_at_ms IS NULL",
        params![session_id, ended_at_ms],
    )?;
    Ok(())
}

pub struct SessionUpsert<'a> {
    pub session_id: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub scope_type: &'a str,
    pub scope_id: &'a str,
    pub board_uid: &'a str,
    pub started_at_ms: i64,
}

pub fn upsert_session(db: &Connection, session: SessionUpsert<'_>) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO record_session(session_id,kind,title,scope_type,scope_id,board_uid,started_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session_id) DO UPDATE SET
         title=excluded.title,board_uid=CASE WHEN record_session.board_uid='' THEN excluded.board_uid ELSE record_session.board_uid END",
        params![session.session_id, session.kind, session.title, session.scope_type,
            session.scope_id, session.board_uid, session.started_at_ms],
    )?;
    Ok(())
}

pub fn list_events(db: &Connection, limit: u32) -> rusqlite::Result<Vec<RecordEvent>> {
    let mut statement = db.prepare(
        "SELECT event_id,occurred_at_ms,actor_type,actor_id,category,action,scope_type,scope_id,
         session_id,correlation_id,causation_id,result,error_code,summary,metadata_json,board_uid
         FROM record_event ORDER BY occurred_at_ms DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit], |row| {
        let metadata: String = row.get(14)?;
        Ok(RecordEvent {
            event_id: row.get(0)?,
            occurred_at_ms: row.get(1)?,
            actor_type: row.get(2)?,
            actor_id: row.get(3)?,
            category: row.get(4)?,
            action: row.get(5)?,
            scope_type: row.get(6)?,
            scope_id: row.get(7)?,
            board_uid: row.get(15)?,
            session_id: row.get(8)?,
            correlation_id: row.get(9)?,
            causation_id: row.get(10)?,
            result: row.get(11)?,
            error_code: row.get(12)?,
            summary: row.get(13)?,
            metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| serde_json::json!({})),
        })
    })?;
    rows.collect()
}

pub fn list_sessions(db: &Connection, limit: u32) -> rusqlite::Result<Vec<RecordSession>> {
    let mut statement = db.prepare(
        "SELECT s.session_id,s.kind,s.title,s.scope_type,s.scope_id,s.board_uid,s.started_at_ms,
         s.ended_at_ms,COUNT(e.event_id),s.integrity FROM record_session s LEFT JOIN record_event e
         ON e.session_id=s.session_id GROUP BY s.session_id ORDER BY s.started_at_ms DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit], |row| {
        Ok(RecordSession {
            session_id: row.get(0)?,
            kind: row.get(1)?,
            title: row.get(2)?,
            scope_type: row.get(3)?,
            scope_id: row.get(4)?,
            board_uid: row.get(5)?,
            started_at_ms: row.get(6)?,
            ended_at_ms: row.get(7)?,
            event_count: row.get(8)?,
            integrity: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn overview(db: &Connection, day_start_ms: i64) -> rusqlite::Result<RecordOverview> {
    let scalar = |sql: &str, value: i64| db.query_row(sql, [value], |row| row.get::<_, u32>(0));
    Ok(RecordOverview {
        today_event_count: scalar("SELECT COUNT(*) FROM record_event WHERE occurred_at_ms>=?1", day_start_ms)?,
        active_serial_sessions: db.query_row("SELECT COUNT(*) FROM record_session WHERE kind='serial' AND ended_at_ms IS NULL", [], |row| row.get(0))?,
        ai_session_count: scalar("SELECT COUNT(*) FROM record_session WHERE kind='ai' AND started_at_ms>=?1", day_start_ms)?,
        device_error_count: scalar("SELECT COUNT(*) FROM record_event WHERE category='device' AND result='failed' AND occurred_at_ms>=?1", day_start_ms)?,
        pending_feedback_count: db.query_row("SELECT COUNT(*) FROM feedback_draft WHERE status='draft'", [], |row| row.get(0))?,
        gap_count: scalar("SELECT COUNT(*) FROM record_event WHERE category='record_gap' AND occurred_at_ms>=?1", day_start_ms)?,
        latest_backup_at_ms: db.query_row("SELECT MAX(created_at_ms) FROM record_backup", [], |row| row.get(0))?,
        lan_service_enabled: false,
    })
}

pub fn save_feedback(
    db: &Connection,
    request: &SaveFeedbackDraftRequest,
    id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO feedback_draft VALUES(?1,?2,?3,?4,?5,?6,?7,'draft',?8,?9,?10,?10) ON CONFLICT(id) DO UPDATE SET title=?2,problem_type=?3,page=?4,expected=?5,actual=?6,reproducible=?7,event_ids_json=?8,include_sensitive_body=?9,updated_at_ms=?10",
        params![id, request.title, request.problem_type, request.page, request.expected, request.actual, request.reproducible, serde_json::to_string(&request.event_ids).unwrap_or_else(|_| "[]".to_owned()), request.include_sensitive_body, now],
    )?;
    Ok(())
}

pub fn list_feedback(db: &Connection) -> rusqlite::Result<Vec<FeedbackDraft>> {
    let mut statement = db.prepare("SELECT id,title,problem_type,page,expected,actual,reproducible,status,event_ids_json,include_sensitive_body,created_at_ms,updated_at_ms FROM feedback_draft ORDER BY updated_at_ms DESC")?;
    let rows = statement.query_map([], |row| {
        let ids: String = row.get(8)?;
        Ok(FeedbackDraft {
            id: row.get(0)?,
            title: row.get(1)?,
            problem_type: row.get(2)?,
            page: row.get(3)?,
            expected: row.get(4)?,
            actual: row.get(5)?,
            reproducible: row.get(6)?,
            status: row.get(7)?,
            event_ids: serde_json::from_str(&ids).unwrap_or_default(),
            include_sensitive_body: row.get(9)?,
            created_at_ms: row.get(10)?,
            updated_at_ms: row.get(11)?,
        })
    })?;
    rows.collect()
}

pub fn insert_backup(db: &Connection, record: &BackupRecord) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO record_backup VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            record.id,
            record.path,
            record.sha256,
            record.event_count,
            record.session_count,
            record.feedback_count,
            record.created_at_ms
        ],
    )?;
    Ok(())
}

pub fn list_backups(db: &Connection) -> rusqlite::Result<Vec<BackupRecord>> {
    let mut statement = db.prepare("SELECT id,path,sha256,event_count,session_count,feedback_count,created_at_ms FROM record_backup ORDER BY created_at_ms DESC LIMIT 50")?;
    let rows = statement.query_map([], |row| {
        Ok(BackupRecord {
            id: row.get(0)?,
            path: row.get(1)?,
            sha256: row.get(2)?,
            event_count: row.get(3)?,
            session_count: row.get(4)?,
            feedback_count: row.get(5)?,
            created_at_ms: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn existing_ids(
    db: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let allowed = matches!(
        (table, column),
        ("record_event", "event_id") | ("record_session", "session_id") | ("feedback_draft", "id")
    );
    if !allowed {
        return Err(rusqlite::Error::InvalidParameterName(table.to_owned()));
    }
    let mut statement = db.prepare(&format!("SELECT {column} FROM {table}"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

pub fn restore_non_overwrite(
    db: &mut Connection,
    events: &[RecordEvent],
    sessions: &[RecordSession],
    feedback: &[FeedbackDraft],
) -> rusqlite::Result<(u32, u32, u32)> {
    let transaction = db.transaction()?;
    let mut imported_sessions = 0;
    for session in sessions {
        imported_sessions += transaction.execute(
            "INSERT OR IGNORE INTO record_session(session_id,kind,title,scope_type,scope_id,board_uid,
             started_at_ms,ended_at_ms,integrity) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![session.session_id,session.kind,session.title,session.scope_type,session.scope_id,
                session.board_uid,session.started_at_ms,session.ended_at_ms,session.integrity],
        )? as u32;
    }
    let mut imported_events = 0;
    for event in events {
        imported_events += transaction.execute(
            "INSERT OR IGNORE INTO record_event(event_id,occurred_at_ms,actor_type,actor_id,category,
             action,scope_type,scope_id,session_id,correlation_id,causation_id,result,error_code,
             summary,metadata_json,board_uid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![event.event_id,event.occurred_at_ms,event.actor_type,event.actor_id,event.category,
                event.action,event.scope_type,event.scope_id,event.session_id,event.correlation_id,
                event.causation_id,event.result,event.error_code,event.summary,event.metadata.to_string(),
                event.board_uid],
        )? as u32;
    }
    let mut imported_feedback = 0;
    for item in feedback {
        imported_feedback += transaction.execute(
            "INSERT OR IGNORE INTO feedback_draft VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                item.id,
                item.title,
                item.problem_type,
                item.page,
                item.expected,
                item.actual,
                item.reproducible,
                item.status,
                serde_json::to_string(&item.event_ids).unwrap_or_else(|_| "[]".to_owned()),
                item.include_sensitive_body,
                item.created_at_ms,
                item.updated_at_ms
            ],
        )? as u32;
    }
    transaction.commit()?;
    Ok((imported_events, imported_sessions, imported_feedback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent_and_empty_queries_work() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        let ledger: Vec<(i64, String)> = {
            let mut statement = db
                .prepare("SELECT version,name FROM lynx_schema_migration ORDER BY version")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(ledger.len(), 2);
        assert_eq!(ledger[1], (2, "add_stable_board_identity_mirror".into()));
        assert!(list_events(&db, 20).unwrap().is_empty());
        assert!(list_sessions(&db, 20).unwrap().is_empty());
        assert_eq!(overview(&db, 0).unwrap().today_event_count, 0);
    }

    #[test]
    fn event_and_session_round_trip_preserves_causality() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        upsert_session(
            &db,
            SessionUpsert {
                session_id: "serial-1",
                kind: "serial",
                title: "T153 · COM7",
                scope_type: "board",
                scope_id: "linux:4",
                board_uid: "",
                started_at_ms: 10,
            },
        )
        .unwrap();
        let event = RecordEvent {
            event_id: "evt-1".to_owned(),
            occurred_at_ms: 11,
            actor_type: "service".to_owned(),
            actor_id: "serial-manager".to_owned(),
            category: "serial".to_owned(),
            action: "serial.connected".to_owned(),
            scope_type: "board".to_owned(),
            scope_id: "linux:4".to_owned(),
            board_uid: String::new(),
            session_id: Some("serial-1".to_owned()),
            correlation_id: "corr-1".to_owned(),
            causation_id: Some("evt-0".to_owned()),
            result: "success".to_owned(),
            error_code: None,
            summary: "T153 串口已连接".to_owned(),
            metadata: serde_json::json!({"port":"COM7","baudRate":115200}),
        };
        insert_event(&db, &event).unwrap();
        end_session(&db, "serial-1", 20).unwrap();

        let saved = list_events(&db, 10).unwrap();
        assert_eq!(saved[0].correlation_id, "corr-1");
        assert_eq!(saved[0].metadata["port"], "COM7");
        let sessions = list_sessions(&db, 10).unwrap();
        assert_eq!(sessions[0].event_count, 1);
        assert_eq!(sessions[0].ended_at_ms, Some(20));
    }

    #[test]
    fn stable_board_identity_sync_backfills_events_and_sessions() {
        let mut records = Connection::open_in_memory().unwrap();
        migrate(&records).unwrap();
        upsert_session(
            &records,
            SessionUpsert {
                session_id: "serial-old",
                kind: "serial",
                title: "T153",
                scope_type: "board",
                scope_id: "linux:1",
                board_uid: "",
                started_at_ms: 10,
            },
        )
        .unwrap();
        let event = RecordEvent {
            event_id: "evt-old".to_owned(),
            occurred_at_ms: 11,
            actor_type: "device".to_owned(),
            actor_id: "linux:1".to_owned(),
            category: "serial".to_owned(),
            action: "serial.rx".to_owned(),
            scope_type: "board".to_owned(),
            scope_id: "linux:1".to_owned(),
            board_uid: String::new(),
            session_id: Some("serial-old".to_owned()),
            correlation_id: "corr-old".to_owned(),
            causation_id: None,
            result: "success".to_owned(),
            error_code: None,
            summary: "legacy".to_owned(),
            metadata: serde_json::json!({}),
        };
        insert_event(&records, &event).unwrap();
        let workbench = Connection::open_in_memory().unwrap();
        crate::workbench::storage::migrate(&workbench).unwrap();
        workbench
            .execute(
                "INSERT INTO devices(board_uid,name) VALUES(?1,'T153')",
                ["linux-22222222222222222222222222222222"],
            )
            .unwrap();

        assert_eq!(sync_board_identities(&mut records, &workbench).unwrap(), 1);
        assert_eq!(sync_board_identities(&mut records, &workbench).unwrap(), 1);
        assert_eq!(
            list_events(&records, 1).unwrap()[0].board_uid,
            "linux-22222222222222222222222222222222"
        );
        assert_eq!(
            list_sessions(&records, 1).unwrap()[0].board_uid,
            "linux-22222222222222222222222222222222"
        );
    }

    #[test]
    fn restore_preserves_uid_and_rejects_known_scope_conflict() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO record_board_identity VALUES('linux:1',?1,unixepoch())",
            ["linux-33333333333333333333333333333333"],
        )
        .unwrap();
        let event = RecordEvent {
            event_id: "evt-restored".to_owned(),
            occurred_at_ms: 12,
            actor_type: "device".to_owned(),
            actor_id: "linux:1".to_owned(),
            category: "device".to_owned(),
            action: "device.state".to_owned(),
            scope_type: "board".to_owned(),
            scope_id: "linux:1".to_owned(),
            board_uid: "linux-33333333333333333333333333333333".to_owned(),
            session_id: None,
            correlation_id: "corr-restored".to_owned(),
            causation_id: None,
            result: "success".to_owned(),
            error_code: None,
            summary: "restored".to_owned(),
            metadata: serde_json::json!({}),
        };
        validate_restored_board_identities(&db, std::slice::from_ref(&event), &[]).unwrap();
        restore_non_overwrite(&mut db, std::slice::from_ref(&event), &[], &[]).unwrap();
        assert_eq!(list_events(&db, 1).unwrap()[0].board_uid, event.board_uid);

        let conflict = RecordEvent {
            board_uid: "linux-44444444444444444444444444444444".to_owned(),
            ..event
        };
        let error = validate_restored_board_identities(&db, &[conflict], &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("RESTORE_BOARD_IDENTITY_CONFLICT:linux:1"));
    }

    #[test]
    fn restore_skips_existing_ids_without_overwriting_them() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let existing = RecordEvent {
            event_id: "evt-existing".to_owned(),
            occurred_at_ms: 10,
            actor_type: "user".to_owned(),
            actor_id: "operator-a".to_owned(),
            category: "ui".to_owned(),
            action: "board.open".to_owned(),
            scope_type: "board".to_owned(),
            scope_id: "linux:4".to_owned(),
            board_uid: String::new(),
            session_id: None,
            correlation_id: "corr-existing".to_owned(),
            causation_id: None,
            result: "success".to_owned(),
            error_code: None,
            summary: "当前数据库中的原始记录".to_owned(),
            metadata: serde_json::json!({}),
        };
        insert_event(&db, &existing).unwrap();
        let conflicting = RecordEvent {
            summary: "备份中不得覆盖的内容".to_owned(),
            ..existing.clone()
        };
        let new_event = RecordEvent {
            event_id: "evt-new".to_owned(),
            summary: "从备份新增的记录".to_owned(),
            ..existing.clone()
        };

        let imported = restore_non_overwrite(&mut db, &[conflicting, new_event], &[], &[]).unwrap();

        assert_eq!(imported, (1, 0, 0));
        let saved = list_events(&db, 10).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            saved
                .iter()
                .find(|event| event.event_id == "evt-existing")
                .unwrap()
                .summary,
            "当前数据库中的原始记录"
        );
    }
}

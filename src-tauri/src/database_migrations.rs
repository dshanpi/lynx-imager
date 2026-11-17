use rusqlite::{params, Connection, OptionalExtension};

pub type MigrationAction = fn(&Connection) -> rusqlite::Result<()>;

#[derive(Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub apply: MigrationAction,
}

#[derive(Debug)]
pub enum MigrationError {
    Sql(rusqlite::Error),
    InvalidPlan(String),
    FutureVersion { found: i64, supported: i64 },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "{error}"),
            Self::InvalidPlan(message) => write!(formatter, "MIGRATION_PLAN_INVALID: {message}"),
            Self::FutureVersion { found, supported } => write!(
                formatter,
                "DATABASE_VERSION_NEWER_THAN_APPLICATION: found {found}, supported {supported}"
            ),
        }
    }
}

impl std::error::Error for MigrationError {}

impl From<rusqlite::Error> for MigrationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub fn apply(
    db: &Connection,
    database_name: &str,
    migrations: &[Migration],
) -> Result<(), MigrationError> {
    validate_plan(migrations)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lynx_schema_migration (\
         version INTEGER PRIMARY KEY,\
         name TEXT NOT NULL UNIQUE,\
         applied_at INTEGER NOT NULL\
         );",
    )?;

    let supported = migrations.last().map_or(0, |migration| migration.version);
    let found = db
        .query_row(
            "SELECT MAX(version) FROM lynx_schema_migration",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(0);
    if found > supported {
        return Err(MigrationError::FutureVersion { found, supported });
    }

    for migration in migrations {
        let recorded_name = db
            .query_row(
                "SELECT name FROM lynx_schema_migration WHERE version=?1",
                [migration.version],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(recorded_name) = recorded_name {
            if recorded_name != migration.name {
                return Err(MigrationError::InvalidPlan(format!(
                    "{database_name} version {} is recorded as {recorded_name}, expected {}",
                    migration.version, migration.name
                )));
            }
            continue;
        }

        let transaction = db.unchecked_transaction()?;
        (migration.apply)(&transaction)?;
        transaction.execute(
            "INSERT INTO lynx_schema_migration(version,name,applied_at) VALUES(?1,?2,unixepoch())",
            params![migration.version, migration.name],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn validate_plan(migrations: &[Migration]) -> Result<(), MigrationError> {
    let mut previous = 0;
    for migration in migrations {
        if migration.version <= previous
            || migration.version <= 0
            || migration.name.trim().is_empty()
        {
            return Err(MigrationError::InvalidPlan(
                "versions must be positive and strictly increasing, with non-empty names".into(),
            ));
        }
        previous = migration.version;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_sample(db: &Connection) -> rusqlite::Result<()> {
        db.execute("CREATE TABLE sample(id INTEGER PRIMARY KEY)", [])?;
        Ok(())
    }

    const MIGRATIONS: &[Migration] = &[Migration {
        version: 1,
        name: "initial_schema",
        apply: create_sample,
    }];

    #[test]
    fn ledger_is_idempotent_and_records_name() {
        let db = Connection::open_in_memory().unwrap();
        apply(&db, "test", MIGRATIONS).unwrap();
        apply(&db, "test", MIGRATIONS).unwrap();
        let row: (i64, String) = db
            .query_row(
                "SELECT version,name FROM lynx_schema_migration",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (1, "initial_schema".into()));
    }

    #[test]
    fn future_database_version_is_rejected_before_schema_changes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE lynx_schema_migration(\
             version INTEGER PRIMARY KEY,name TEXT NOT NULL UNIQUE,applied_at INTEGER NOT NULL);\
             INSERT INTO lynx_schema_migration VALUES(99,'future',0);",
        )
        .unwrap();
        let error = apply(&db, "test", MIGRATIONS).unwrap_err().to_string();
        assert!(error.contains("DATABASE_VERSION_NEWER_THAN_APPLICATION"));
        assert!(!db.prepare("SELECT 1 FROM sample").is_ok());
    }
}

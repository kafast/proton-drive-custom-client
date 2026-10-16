//! Database schema and migrations.

use rusqlite::Connection;

/// Migrations in order; index + 1 is the schema version afterwards.
/// Never change existing entries, only append new ones.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE base (
        pair            TEXT NOT NULL,
        path            TEXT NOT NULL,
        remote_id       TEXT NOT NULL,
        remote_revision TEXT,
        local_size      INTEGER,
        local_modified  INTEGER,
        local_hash      TEXT,
        PRIMARY KEY (pair, path)
    );
    CREATE TABLE event_cursors (
        pair   TEXT PRIMARY KEY,
        cursor TEXT NOT NULL
    );
    CREATE TABLE conflicts (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        pair          TEXT NOT NULL,
        path          TEXT NOT NULL,
        kind          TEXT NOT NULL,
        conflict_copy TEXT,
        detected_at   INTEGER NOT NULL
    );
    CREATE TABLE trash (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        pair          TEXT NOT NULL,
        original_path TEXT NOT NULL,
        origin        TEXT NOT NULL,
        locator       TEXT NOT NULL,
        trashed_at    INTEGER NOT NULL
    );
",
    "
    CREATE TABLE job_runs (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        job         TEXT NOT NULL,
        started_at  INTEGER NOT NULL,
        finished_at INTEGER NOT NULL,
        status      TEXT NOT NULL,
        uploaded    INTEGER NOT NULL,
        unchanged   INTEGER NOT NULL,
        expired     INTEGER NOT NULL,
        attempt     INTEGER NOT NULL,
        message     TEXT
    );
    CREATE INDEX job_runs_by_job ON job_runs (job, id);
",
    "
    CREATE TABLE cache_records (
        drive       TEXT NOT NULL,
        key         INTEGER NOT NULL,
        node        TEXT,
        path        TEXT NOT NULL,
        revision    TEXT,
        size        INTEGER NOT NULL,
        blocks      BLOB NOT NULL,
        last_access INTEGER NOT NULL,
        modified    INTEGER NOT NULL,
        changed     INTEGER NOT NULL,
        PRIMARY KEY (drive, key)
    );
    CREATE TABLE pin_rules (
        drive TEXT NOT NULL,
        path  TEXT NOT NULL,
        state TEXT NOT NULL,
        PRIMARY KEY (drive, path)
    );
",
];

/// Sets the connection options and brings the schema up to date.
pub(crate) fn prepare(connection: &Connection) -> rusqlite::Result<()> {
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;

    let current: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = usize::try_from(current).unwrap_or(usize::MAX);
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        let version = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let connection = Connection::open_in_memory().unwrap();
        prepare(&connection).unwrap();
        prepare(&connection).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }
}

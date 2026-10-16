//! Datenbankschema und Migrationen.

use rusqlite::Connection;

/// Migrationen in Reihenfolge; Index + 1 ist die Schemaversion danach.
/// Bestehende Einträge nie ändern, nur neue anhängen.
const MIGRATIONS: &[&str] = &["
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
"];

/// Setzt die Verbindungsoptionen und bringt das Schema auf den neuesten Stand.
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

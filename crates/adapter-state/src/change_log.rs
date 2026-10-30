//! Change log of the sync pairs.

use protonsync_application::ports::{ChangeLog, ChangeQuery, LoggedChange, StateError};
use protonsync_domain::change_log::{ChangeEntry, ChangeKind, ChangedSide};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use rusqlite::{Row, params};

use super::{SqliteStateRepo, corrupt, parse_path, unavailable};

const fn side_to_text(side: ChangedSide) -> &'static str {
    match side {
        ChangedSide::Local => "local",
        ChangedSide::Remote => "remote",
    }
}

fn side_from_text(text: &str) -> Result<ChangedSide, StateError> {
    match text {
        "local" => Ok(ChangedSide::Local),
        "remote" => Ok(ChangedSide::Remote),
        other => Err(corrupt(format!("unknown change side {other}"))),
    }
}

const fn kind_to_text(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Created => "created",
        ChangeKind::Modified => "modified",
        ChangeKind::Trashed => "trashed",
        ChangeKind::Archived => "archived",
        ChangeKind::ArchivedAndReplaced => "archived_and_replaced",
        ChangeKind::KeptCopy => "kept_copy",
        ChangeKind::Conflict => "conflict",
    }
}

fn kind_from_text(text: &str) -> Result<ChangeKind, StateError> {
    match text {
        "created" => Ok(ChangeKind::Created),
        "modified" => Ok(ChangeKind::Modified),
        "trashed" => Ok(ChangeKind::Trashed),
        "archived" => Ok(ChangeKind::Archived),
        "archived_and_replaced" => Ok(ChangeKind::ArchivedAndReplaced),
        "kept_copy" => Ok(ChangeKind::KeptCopy),
        "conflict" => Ok(ChangeKind::Conflict),
        other => Err(corrupt(format!("unknown change kind {other}"))),
    }
}

/// A row from `changes`, as stored.
struct ChangeRow {
    pair: String,
    at: i64,
    path: String,
    side: String,
    kind: String,
    is_folder: bool,
}

impl ChangeRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            pair: row.get(0)?,
            at: row.get(1)?,
            path: row.get(2)?,
            side: row.get(3)?,
            kind: row.get(4)?,
            is_folder: row.get(5)?,
        })
    }

    fn into_logged(self) -> Result<LoggedChange, StateError> {
        Ok(LoggedChange {
            pair: SyncPairId::new(self.pair),
            change: ChangeEntry {
                at: Timestamp::from_unix_seconds(self.at),
                path: parse_path(&self.path)?,
                side: side_from_text(&self.side)?,
                kind: kind_from_text(&self.kind)?,
                is_folder: self.is_folder,
            },
        })
    }
}

impl ChangeLog for SqliteStateRepo {
    fn record_changes(&self, pair: &SyncPairId, changes: &[ChangeEntry]) -> Result<(), StateError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(unavailable)?;
        {
            // The statement borrows the transaction; it must be dropped before `commit`.
            let mut insert = transaction
                .prepare(
                    "INSERT INTO changes (pair, at, path, side, kind, is_folder)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(unavailable)?;
            for change in changes {
                insert
                    .execute(params![
                        pair.as_str(),
                        change.at.unix_seconds(),
                        change.path.as_str(),
                        side_to_text(change.side),
                        kind_to_text(change.kind),
                        change.is_folder,
                    ])
                    .map_err(unavailable)?;
            }
        }
        transaction.commit().map_err(unavailable)
    }

    fn changes(&self, query: &ChangeQuery) -> Result<Vec<LoggedChange>, StateError> {
        let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);
        let pair = query.pair.as_ref().map(SyncPairId::as_str);
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT pair, at, path, side, kind, is_folder FROM changes
                 WHERE at >= ?1 AND (?2 IS NULL OR pair = ?2)
                 ORDER BY at DESC, id DESC LIMIT ?3",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map(
                params![query.since.unix_seconds(), pair, limit],
                ChangeRow::read,
            )
            .map_err(unavailable)?;
        rows.map(|row| row.map_err(unavailable)?.into_logged())
            .collect()
    }

    fn prune_changes(&self, before: Timestamp) -> Result<usize, StateError> {
        self.connection()?
            .execute(
                "DELETE FROM changes WHERE at < ?1",
                params![before.unix_seconds()],
            )
            .map_err(unavailable)
    }
}

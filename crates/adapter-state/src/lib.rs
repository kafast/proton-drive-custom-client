//! SQLite-Zustandsspeicher (Adapter für die Ports [`StateRepo`], [`JobLog`] und
//! [`DriveRepo`](protonsync_application::ports::DriveRepo)).
//!
//! Die Datenbank läuft im WAL-Modus mit `synchronous = FULL`: Jede bestätigte
//! Änderung übersteht auch einen Absturz oder Stromausfall. Das Schema wird
//! über `PRAGMA user_version` versioniert und beim Öffnen migriert.

mod drive;
mod schema;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{
    BaseEntry, ConflictId, EventCursor, JOB_HISTORY_LIMIT, JobLog, StateError, StateRepo, TrashId,
};
use protonsync_domain::conflict::{Conflict, ConflictKind};
use protonsync_domain::job::{JobId, JobOutcome, JobRun};
use protonsync_domain::node::{Fingerprint, NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Zustandsspeicher in einer SQLite-Datei.
#[derive(Debug)]
pub struct SqliteStateRepo {
    // `rusqlite::Connection` darf nicht von mehreren Threads gleichzeitig
    // benutzt werden; der Mutex sorgt dafür (wie `synchronized` in Java).
    connection: Mutex<Connection>,
}

impl SqliteStateRepo {
    /// Öffnet (oder erzeugt) die Datenbank unter `path`. Neue Dateien sind nur
    /// für den eigenen Benutzer lesbar.
    ///
    /// # Errors
    /// [`StateError::Unavailable`], wenn die Datei nicht geöffnet oder migriert werden kann.
    pub fn open(path: &Path) -> Result<Self, StateError> {
        let is_new = !path.exists();
        let connection = Connection::open(path).map_err(unavailable)?;
        if is_new {
            restrict_permissions(path)?;
        }
        Self::with_connection(connection)
    }

    /// Datenbank nur im Arbeitsspeicher (für Tests).
    ///
    /// # Errors
    /// [`StateError::Unavailable`], wenn SQLite nicht initialisiert werden kann.
    pub fn in_memory() -> Result<Self, StateError> {
        Self::with_connection(Connection::open_in_memory().map_err(unavailable)?)
    }

    fn with_connection(connection: Connection) -> Result<Self, StateError> {
        schema::prepare(&connection).map_err(unavailable)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StateError> {
        self.connection
            .lock()
            .map_err(|_| StateError::Unavailable("Mutex vergiftet".into()))
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_READ_WRITE: u32 = 0o600;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_READ_WRITE))
        .map_err(|error| StateError::Unavailable(error.to_string()))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

#[allow(clippy::needless_pass_by_value)] // Signatur passt zu `map_err`.
fn unavailable(error: rusqlite::Error) -> StateError {
    StateError::Unavailable(error.to_string())
}

fn corrupt(what: impl std::fmt::Display) -> StateError {
    StateError::Corrupt(what.to_string())
}

fn to_i64(value: u64) -> Result<i64, StateError> {
    i64::try_from(value).map_err(|_| corrupt(format!("Wert {value} zu groß")))
}

fn parse_path(value: &str) -> Result<RelativePath, StateError> {
    RelativePath::parse(value).map_err(corrupt)
}

/// Escaped `%`, `_` und `\` für ein `LIKE ... ESCAPE '\'`.
fn like_prefix(path: &RelativePath) -> String {
    let escaped = path
        .as_str()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}/%")
}

const KIND_BOTH_MODIFIED: &str = "both_modified";
const KIND_LOCAL_MODIFIED_REMOTE_DELETED: &str = "local_modified_remote_deleted";
const KIND_LOCAL_DELETED_REMOTE_MODIFIED: &str = "local_deleted_remote_modified";
const ORIGIN_LOCAL: &str = "local";
const ORIGIN_REMOTE: &str = "remote";

const fn kind_to_text(kind: ConflictKind) -> &'static str {
    match kind {
        ConflictKind::BothModified => KIND_BOTH_MODIFIED,
        ConflictKind::LocalModifiedRemoteDeleted => KIND_LOCAL_MODIFIED_REMOTE_DELETED,
        ConflictKind::LocalDeletedRemoteModified => KIND_LOCAL_DELETED_REMOTE_MODIFIED,
    }
}

fn kind_from_text(text: &str) -> Result<ConflictKind, StateError> {
    match text {
        KIND_BOTH_MODIFIED => Ok(ConflictKind::BothModified),
        KIND_LOCAL_MODIFIED_REMOTE_DELETED => Ok(ConflictKind::LocalModifiedRemoteDeleted),
        KIND_LOCAL_DELETED_REMOTE_MODIFIED => Ok(ConflictKind::LocalDeletedRemoteModified),
        other => Err(corrupt(format!("unbekannte Konfliktart {other}"))),
    }
}

/// Rohdaten einer Zeile der Tabelle `base`, bevor sie geprüft werden.
struct BaseRow {
    path: String,
    remote_id: String,
    remote_revision: Option<String>,
    local_size: Option<i64>,
    local_modified: Option<i64>,
    local_hash: Option<String>,
}

impl BaseRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            path: row.get(0)?,
            remote_id: row.get(1)?,
            remote_revision: row.get(2)?,
            local_size: row.get(3)?,
            local_modified: row.get(4)?,
            local_hash: row.get(5)?,
        })
    }

    fn into_entry(self) -> Result<BaseEntry, StateError> {
        let local = match (self.local_size, self.local_modified) {
            (Some(size), Some(modified)) => Some(Fingerprint {
                size: u64::try_from(size).map_err(|_| corrupt("negative Größe"))?,
                modified: Timestamp::from_unix_seconds(modified),
                content_hash: self.local_hash,
            }),
            (None, None) => None,
            _ => return Err(corrupt("unvollständiger lokaler Stand")),
        };
        Ok(BaseEntry {
            path: parse_path(&self.path)?,
            remote_id: NodeId::new(self.remote_id),
            remote_revision: self.remote_revision.map(RevisionId::new),
            local,
        })
    }
}

impl StateRepo for SqliteStateRepo {
    fn base_entries(&self, pair: &SyncPairId) -> Result<Vec<BaseEntry>, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT path, remote_id, remote_revision, local_size, local_modified, local_hash
                 FROM base WHERE pair = ?1 ORDER BY path",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map([pair.as_str()], BaseRow::read)
            .map_err(unavailable)?;
        rows.map(|row| row.map_err(unavailable)?.into_entry())
            .collect()
    }

    fn save_base(&self, pair: &SyncPairId, entry: &BaseEntry) -> Result<(), StateError> {
        let local = entry.local.as_ref();
        let size = local.map(|fp| to_i64(fp.size)).transpose()?;
        self.connection()?
            .execute(
                "INSERT INTO base (pair, path, remote_id, remote_revision, local_size, local_modified, local_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (pair, path) DO UPDATE SET
                   remote_id = excluded.remote_id, remote_revision = excluded.remote_revision,
                   local_size = excluded.local_size, local_modified = excluded.local_modified,
                   local_hash = excluded.local_hash",
                params![
                    pair.as_str(),
                    entry.path.as_str(),
                    entry.remote_id.as_str(),
                    entry.remote_revision.as_ref().map(RevisionId::as_str),
                    size,
                    local.map(|fp| fp.modified.unix_seconds()),
                    local.and_then(|fp| fp.content_hash.as_deref()),
                ],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    fn remove_base_tree(&self, pair: &SyncPairId, path: &RelativePath) -> Result<(), StateError> {
        let connection = self.connection()?;
        if path.is_root() {
            connection
                .execute("DELETE FROM base WHERE pair = ?1", [pair.as_str()])
                .map_err(unavailable)?;
        } else {
            connection
                .execute(
                    "DELETE FROM base WHERE pair = ?1 AND (path = ?2 OR path LIKE ?3 ESCAPE '\\')",
                    params![pair.as_str(), path.as_str(), like_prefix(path)],
                )
                .map_err(unavailable)?;
        }
        Ok(())
    }

    fn event_cursor(&self, pair: &SyncPairId) -> Result<Option<EventCursor>, StateError> {
        let cursor: Option<String> = self
            .connection()?
            .query_row(
                "SELECT cursor FROM event_cursors WHERE pair = ?1",
                [pair.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)?;
        Ok(cursor.map(EventCursor::new))
    }

    fn save_event_cursor(&self, pair: &SyncPairId, cursor: &EventCursor) -> Result<(), StateError> {
        self.connection()?
            .execute(
                "INSERT INTO event_cursors (pair, cursor) VALUES (?1, ?2)
                 ON CONFLICT (pair) DO UPDATE SET cursor = excluded.cursor",
                params![pair.as_str(), cursor.as_str()],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    fn add_conflict(
        &self,
        pair: &SyncPairId,
        conflict: &Conflict,
    ) -> Result<ConflictId, StateError> {
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO conflicts (pair, path, kind, conflict_copy, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    pair.as_str(),
                    conflict.relative_path.as_str(),
                    kind_to_text(conflict.kind),
                    conflict.conflict_copy.as_ref().map(RelativePath::as_str),
                    conflict.detected_at.unix_seconds(),
                ],
            )
            .map_err(unavailable)?;
        Ok(ConflictId(connection.last_insert_rowid()))
    }

    fn conflicts(&self, pair: &SyncPairId) -> Result<Vec<(ConflictId, Conflict)>, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, kind, conflict_copy, detected_at
                 FROM conflicts WHERE pair = ?1 ORDER BY id",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map([pair.as_str()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(unavailable)?;
        rows.map(|row| {
            let (id, path, kind, copy, detected_at) = row.map_err(unavailable)?;
            let conflict = Conflict {
                relative_path: parse_path(&path)?,
                kind: kind_from_text(&kind)?,
                conflict_copy: copy.as_deref().map(parse_path).transpose()?,
                detected_at: Timestamp::from_unix_seconds(detected_at),
            };
            Ok((ConflictId(id), conflict))
        })
        .collect()
    }

    fn remove_conflict(&self, id: ConflictId) -> Result<(), StateError> {
        self.connection()?
            .execute("DELETE FROM conflicts WHERE id = ?1", [id.0])
            .map_err(unavailable)?;
        Ok(())
    }

    fn add_trash(&self, pair: &SyncPairId, entry: &TrashEntry) -> Result<TrashId, StateError> {
        let (origin, locator) = match &entry.origin {
            TrashOrigin::Local { stored_as } => (ORIGIN_LOCAL, stored_as.as_str()),
            TrashOrigin::Remote { node } => (ORIGIN_REMOTE, node.as_str()),
        };
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO trash (pair, original_path, origin, locator, trashed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    pair.as_str(),
                    entry.original_path.as_str(),
                    origin,
                    locator,
                    entry.trashed_at.unix_seconds(),
                ],
            )
            .map_err(unavailable)?;
        Ok(TrashId(connection.last_insert_rowid()))
    }

    fn trash_entries(&self, pair: &SyncPairId) -> Result<Vec<(TrashId, TrashEntry)>, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, original_path, origin, locator, trashed_at
                 FROM trash WHERE pair = ?1 ORDER BY id",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map([pair.as_str()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(unavailable)?;
        rows.map(|row| {
            let (id, path, origin, locator, trashed_at) = row.map_err(unavailable)?;
            let origin = match origin.as_str() {
                ORIGIN_LOCAL => TrashOrigin::Local { stored_as: locator },
                ORIGIN_REMOTE => TrashOrigin::Remote {
                    node: NodeId::new(locator),
                },
                other => return Err(corrupt(format!("unbekannte Herkunft {other}"))),
            };
            let entry = TrashEntry {
                original_path: parse_path(&path)?,
                origin,
                trashed_at: Timestamp::from_unix_seconds(trashed_at),
            };
            Ok((TrashId(id), entry))
        })
        .collect()
    }

    fn remove_trash(&self, id: TrashId) -> Result<(), StateError> {
        self.connection()?
            .execute("DELETE FROM trash WHERE id = ?1", [id.0])
            .map_err(unavailable)?;
        Ok(())
    }
}

const STATUS_PLANNED: &str = "planned";
const STATUS_SUCCEEDED: &str = "succeeded";
const STATUS_FAILED: &str = "failed";

/// Eine Zeile aus `job_runs`, wie sie gespeichert ist.
struct JobRunRow {
    started_at: i64,
    finished_at: i64,
    status: String,
    uploaded: u32,
    unchanged: u32,
    expired: u32,
    attempt: u32,
    message: Option<String>,
}

impl JobRunRow {
    fn from_run(run: &JobRun) -> Self {
        let (status, uploaded, unchanged, expired, attempt, message) = match &run.outcome {
            JobOutcome::Planned { uploads, expired } => {
                (STATUS_PLANNED, *uploads, 0, *expired, 0, None)
            }
            JobOutcome::Succeeded {
                uploaded,
                unchanged,
                expired,
            } => (STATUS_SUCCEEDED, *uploaded, *unchanged, *expired, 0, None),
            JobOutcome::Failed { message, attempt } => {
                (STATUS_FAILED, 0, 0, 0, *attempt, Some(message.clone()))
            }
        };
        Self {
            started_at: run.started_at.unix_seconds(),
            finished_at: run.finished_at.unix_seconds(),
            status: status.to_owned(),
            uploaded,
            unchanged,
            expired,
            attempt,
            message,
        }
    }

    fn into_run(self) -> Result<JobRun, StateError> {
        let outcome = match self.status.as_str() {
            STATUS_PLANNED => JobOutcome::Planned {
                uploads: self.uploaded,
                expired: self.expired,
            },
            STATUS_SUCCEEDED => JobOutcome::Succeeded {
                uploaded: self.uploaded,
                unchanged: self.unchanged,
                expired: self.expired,
            },
            STATUS_FAILED => JobOutcome::Failed {
                message: self.message.unwrap_or_default(),
                attempt: self.attempt,
            },
            other => return Err(corrupt(format!("unbekannter Job-Status {other}"))),
        };
        Ok(JobRun {
            started_at: Timestamp::from_unix_seconds(self.started_at),
            finished_at: Timestamp::from_unix_seconds(self.finished_at),
            outcome,
        })
    }
}

impl JobLog for SqliteStateRepo {
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError> {
        let row = JobRunRow::from_run(run);
        let limit = i64::try_from(JOB_HISTORY_LIMIT).unwrap_or(i64::MAX);
        let mut connection = self.connection()?;
        // Einfügen und Kürzen gemeinsam: Entweder beides oder nichts.
        let transaction = connection.transaction().map_err(unavailable)?;
        transaction
            .execute(
                "INSERT INTO job_runs
                   (job, started_at, finished_at, status, uploaded, unchanged, expired, attempt, message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    job.as_str(),
                    row.started_at,
                    row.finished_at,
                    row.status,
                    row.uploaded,
                    row.unchanged,
                    row.expired,
                    row.attempt,
                    row.message,
                ],
            )
            .map_err(unavailable)?;
        transaction
            .execute(
                "DELETE FROM job_runs WHERE job = ?1 AND id NOT IN
                   (SELECT id FROM job_runs WHERE job = ?1 ORDER BY id DESC LIMIT ?2)",
                params![job.as_str(), limit],
            )
            .map_err(unavailable)?;
        transaction.commit().map_err(unavailable)
    }

    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT started_at, finished_at, status, uploaded, unchanged, expired, attempt, message
                 FROM job_runs WHERE job = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map(params![job.as_str(), limit], |row| {
                Ok(JobRunRow {
                    started_at: row.get(0)?,
                    finished_at: row.get(1)?,
                    status: row.get(2)?,
                    uploaded: row.get(3)?,
                    unchanged: row.get(4)?,
                    expired: row.get(5)?,
                    attempt: row.get(6)?,
                    message: row.get(7)?,
                })
            })
            .map_err(unavailable)?;
        rows.map(|row| row.map_err(unavailable)?.into_run())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_prefix_escapes_wildcards() {
        let path = RelativePath::parse("a_b%c").unwrap();
        assert_eq!(like_prefix(&path), "a\\_b\\%c/%");
    }

    #[test]
    fn wildcard_names_do_not_match_other_folders() {
        let repo = SqliteStateRepo::in_memory().unwrap();
        let pair = SyncPairId::new("p");
        let entry = |at: &str| BaseEntry {
            path: RelativePath::parse(at).unwrap(),
            remote_id: NodeId::new(at),
            remote_revision: None,
            local: None,
        };
        repo.save_base(&pair, &entry("a_")).unwrap();
        repo.save_base(&pair, &entry("ab/x")).unwrap();
        repo.remove_base_tree(&pair, &RelativePath::parse("a_").unwrap())
            .unwrap();
        assert_eq!(repo.base_entries(&pair).unwrap(), [entry("ab/x")]);
    }
}

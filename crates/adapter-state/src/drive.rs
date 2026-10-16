//! Index des Zwischenspeichers und Anheft-Regeln des virtuellen Laufwerks.

use protonsync_application::ports::{CacheKey, CacheRecord, DriveRepo, StateError};
use protonsync_domain::cache::BlockMap;
use protonsync_domain::node::{NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{PinRules, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use rusqlite::{Row, params};

use super::{SqliteStateRepo, corrupt, parse_path, to_i64, unavailable};

const PIN_PINNED: &str = "pinned";
const PIN_UNPINNED: &str = "unpinned";

const fn pin_to_text(state: PinState) -> &'static str {
    match state {
        PinState::Pinned => PIN_PINNED,
        PinState::Unpinned => PIN_UNPINNED,
    }
}

fn pin_from_text(text: &str) -> Result<PinState, StateError> {
    match text {
        PIN_PINNED => Ok(PinState::Pinned),
        PIN_UNPINNED => Ok(PinState::Unpinned),
        other => Err(corrupt(format!("unbekannte Anheft-Regel {other}"))),
    }
}

fn to_u64(value: i64) -> Result<u64, StateError> {
    u64::try_from(value).map_err(|_| corrupt(format!("negativer Wert {value}")))
}

/// Eine Zeile aus `cache_records`, wie sie gespeichert ist.
struct CacheRow {
    key: i64,
    node: Option<String>,
    path: String,
    revision: Option<String>,
    size: i64,
    blocks: Vec<u8>,
    last_access: i64,
    modified: i64,
    changed: bool,
}

impl CacheRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            key: row.get(0)?,
            node: row.get(1)?,
            path: row.get(2)?,
            revision: row.get(3)?,
            size: row.get(4)?,
            blocks: row.get(5)?,
            last_access: row.get(6)?,
            modified: row.get(7)?,
            changed: row.get(8)?,
        })
    }

    fn into_record(self) -> Result<CacheRecord, StateError> {
        Ok(CacheRecord {
            key: CacheKey(to_u64(self.key)?),
            node: self.node.map(NodeId::new),
            path: parse_path(&self.path)?,
            revision: self.revision.map(RevisionId::new),
            blocks: BlockMap::from_bytes(to_u64(self.size)?, self.blocks),
            last_access: Timestamp::from_unix_seconds(self.last_access),
            modified: Timestamp::from_unix_seconds(self.modified),
            changed: self.changed,
        })
    }
}

impl DriveRepo for SqliteStateRepo {
    fn cache_records(&self, drive: &SyncPairId) -> Result<Vec<CacheRecord>, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT key, node, path, revision, size, blocks, last_access, modified, changed
                 FROM cache_records WHERE drive = ?1 ORDER BY key",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map([drive.as_str()], CacheRow::read)
            .map_err(unavailable)?;
        rows.map(|row| row.map_err(unavailable)?.into_record())
            .collect()
    }

    fn save_cache_record(
        &self,
        drive: &SyncPairId,
        record: &CacheRecord,
    ) -> Result<(), StateError> {
        self.connection()?
            .execute(
                "INSERT OR REPLACE INTO cache_records
                   (drive, key, node, path, revision, size, blocks, last_access, modified, changed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    drive.as_str(),
                    to_i64(record.key.0)?,
                    record.node.as_ref().map(NodeId::as_str),
                    record.path.as_str(),
                    record.revision.as_ref().map(RevisionId::as_str),
                    to_i64(record.blocks.size())?,
                    record.blocks.as_bytes(),
                    record.last_access.unix_seconds(),
                    record.modified.unix_seconds(),
                    record.changed,
                ],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    fn remove_cache_record(&self, drive: &SyncPairId, key: CacheKey) -> Result<(), StateError> {
        self.connection()?
            .execute(
                "DELETE FROM cache_records WHERE drive = ?1 AND key = ?2",
                params![drive.as_str(), to_i64(key.0)?],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    fn pin_rules(&self, drive: &SyncPairId) -> Result<PinRules, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT path, state FROM pin_rules WHERE drive = ?1")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([drive.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(unavailable)?;
        let mut rules = PinRules::new();
        for row in rows {
            let (path, state) = row.map_err(unavailable)?;
            rules.set(parse_path(&path)?, pin_from_text(&state)?);
        }
        Ok(rules)
    }

    fn save_pin_rule(
        &self,
        drive: &SyncPairId,
        path: &RelativePath,
        state: Option<PinState>,
    ) -> Result<(), StateError> {
        let connection = self.connection()?;
        match state {
            Some(state) => connection.execute(
                "INSERT OR REPLACE INTO pin_rules (drive, path, state) VALUES (?1, ?2, ?3)",
                params![drive.as_str(), path.as_str(), pin_to_text(state)],
            ),
            None => connection.execute(
                "DELETE FROM pin_rules WHERE drive = ?1 AND path = ?2",
                params![drive.as_str(), path.as_str()],
            ),
        }
        .map_err(unavailable)?;
        Ok(())
    }
}

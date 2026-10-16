//! Der SQLite-Speicher besteht die Contract-Tests für `StateRepo`, im Speicher
//! und als echte Datei.

// Hilfsfunktionen in Testdateien dürfen wie Tests abbrechen.
#![allow(clippy::unwrap_used)]

use protonsync_adapter_state::SqliteStateRepo;

mod in_memory {
    use super::SqliteStateRepo;

    protonsync_testkit::state_repo_contract_tests!(|| SqliteStateRepo::in_memory().unwrap());
}

mod on_disk {
    use super::SqliteStateRepo;

    /// Hält das temporäre Verzeichnis am Leben, solange der Speicher benutzt wird.
    struct OnDisk {
        repo: SqliteStateRepo,
        _dir: tempfile::TempDir,
    }

    impl std::ops::Deref for OnDisk {
        type Target = SqliteStateRepo;
        fn deref(&self) -> &SqliteStateRepo {
            &self.repo
        }
    }

    fn open() -> OnDisk {
        let dir = tempfile::tempdir().unwrap();
        let repo = SqliteStateRepo::open(&dir.path().join("state.db")).unwrap();
        OnDisk { repo, _dir: dir }
    }

    protonsync_testkit::state_repo_contract_tests!(deref: open);
}

#[test]
fn state_survives_reopen() {
    use protonsync_application::ports::{EventCursor, StateRepo};
    use protonsync_domain::sync_pair::SyncPairId;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.db");
    let pair = SyncPairId::new("p");
    SqliteStateRepo::open(&file)
        .unwrap()
        .save_event_cursor(&pair, &EventCursor::new("42"))
        .unwrap();
    let reopened = SqliteStateRepo::open(&file).unwrap();
    assert_eq!(
        reopened.event_cursor(&pair).unwrap(),
        Some(EventCursor::new("42"))
    );
}

#[cfg(unix)]
#[test]
fn new_database_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.db");
    SqliteStateRepo::open(&file).unwrap();
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

mod job_log {
    use super::SqliteStateRepo;

    protonsync_testkit::job_log_contract_tests!(|| SqliteStateRepo::in_memory().unwrap());
}

mod drive_repo {
    use super::SqliteStateRepo;

    protonsync_testkit::drive_repo_contract_tests!(|| SqliteStateRepo::in_memory().unwrap());
}

/// Der Index des Zwischenspeichers übersteht einen Neustart (Wiederaufnahme von Uploads).
#[test]
fn cache_records_survive_reopen() {
    use protonsync_application::ports::{CacheKey, CacheRecord, DriveRepo};
    use protonsync_domain::cache::BlockMap;
    use protonsync_domain::path::RelativePath;
    use protonsync_domain::pin::PinState;
    use protonsync_domain::sync_pair::SyncPairId;
    use protonsync_domain::time::Timestamp;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.db");
    let drive = SyncPairId::new("laufwerk");
    let record = CacheRecord {
        key: CacheKey(7),
        node: None,
        path: RelativePath::parse("Neu.txt").unwrap(),
        revision: None,
        blocks: BlockMap::complete(3),
        last_access: Timestamp::from_unix_seconds(5),
        modified: Timestamp::from_unix_seconds(5),
        changed: true,
    };
    {
        let repo = SqliteStateRepo::open(&file).unwrap();
        repo.save_cache_record(&drive, &record).unwrap();
        repo.save_pin_rule(&drive, &record.path, Some(PinState::Pinned))
            .unwrap();
    }
    let reopened = SqliteStateRepo::open(&file).unwrap();
    assert_eq!(
        reopened.cache_records(&drive).unwrap(),
        vec![record.clone()]
    );
    assert_eq!(
        reopened
            .pin_rules(&drive)
            .unwrap()
            .effective(&record.path, PinState::Unpinned),
        PinState::Pinned
    );
}

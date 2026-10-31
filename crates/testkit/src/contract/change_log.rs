//! Contract tests for [`ChangeLog`].
//!
//! ```ignore
//! protonsync_testkit::change_log_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
//! ```

use protonsync_application::ports::{ChangeLog, ChangeQuery};
use protonsync_domain::change_log::{ChangeEntry, ChangeKind, ChangedSide};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

fn change(at: i64, path: &str, side: ChangedSide, kind: ChangeKind) -> ChangeEntry {
    ChangeEntry {
        at: Timestamp::from_unix_seconds(at),
        path: RelativePath::parse(path).unwrap(),
        side,
        kind,
        is_folder: false,
    }
}

fn all_since(since: i64) -> ChangeQuery {
    ChangeQuery {
        since: Timestamp::from_unix_seconds(since),
        pair: None,
        limit: usize::MAX,
    }
}

/// Every side and kind comes back unchanged, newest first, with its pair.
pub fn changes_roundtrip(log: &impl ChangeLog) {
    let home = SyncPairId::new("home");
    let kinds = [
        ChangeKind::Created,
        ChangeKind::Modified,
        ChangeKind::Trashed,
        ChangeKind::Archived,
        ChangeKind::ArchivedAndReplaced,
        ChangeKind::KeptCopy,
        ChangeKind::Conflict,
    ];
    let mut written = Vec::new();
    for (index, kind) in (0_i64..).zip(kinds) {
        for side in [ChangedSide::Local, ChangedSide::Remote] {
            written.push(change(index, &format!("dir/{index}.txt"), side, kind));
        }
    }
    let mut folder = change(100, "dir", ChangedSide::Remote, ChangeKind::Created);
    folder.is_folder = true;
    written.push(folder);
    log.record_changes(&home, &written).unwrap();

    let read = log.changes(&all_since(0)).unwrap();
    assert_eq!(read.len(), written.len());
    assert!(read.iter().all(|logged| logged.pair == home));
    let mut expected = written;
    expected.reverse();
    let read: Vec<ChangeEntry> = read.into_iter().map(|logged| logged.change).collect();
    assert_eq!(read, expected);
}

/// The query filters by time and pair and keeps only the newest `limit`.
pub fn changes_are_filtered(log: &impl ChangeLog) {
    let home = SyncPairId::new("home");
    let work = SyncPairId::new("work");
    let modified = |at| change(at, "a.txt", ChangedSide::Remote, ChangeKind::Modified);
    log.record_changes(&home, &[modified(10), modified(20)])
        .unwrap();
    log.record_changes(&work, &[modified(15), modified(30)])
        .unwrap();
    log.record_changes(&home, &[]).unwrap();

    let since_15: Vec<i64> = log
        .changes(&all_since(15))
        .unwrap()
        .iter()
        .map(|logged| logged.change.at.unix_seconds())
        .collect();
    assert_eq!(since_15, vec![30, 20, 15]);

    let only_home = log
        .changes(&ChangeQuery {
            pair: Some(home.clone()),
            ..all_since(0)
        })
        .unwrap();
    assert_eq!(only_home.len(), 2);
    assert!(only_home.iter().all(|logged| logged.pair == home));

    let newest = log
        .changes(&ChangeQuery {
            limit: 1,
            ..all_since(0)
        })
        .unwrap();
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].pair, work);
    assert_eq!(newest[0].change.at.unix_seconds(), 30);
}

/// Pruning removes exactly the entries older than the limit.
pub fn old_changes_are_pruned(log: &impl ChangeLog) {
    let home = SyncPairId::new("home");
    let trashed = |at| change(at, "a.txt", ChangedSide::Local, ChangeKind::Trashed);
    log.record_changes(&home, &[trashed(5), trashed(10), trashed(11)])
        .unwrap();

    let removed = log.prune_changes(Timestamp::from_unix_seconds(10)).unwrap();

    assert_eq!(removed, 1);
    let left: Vec<i64> = log
        .changes(&all_since(0))
        .unwrap()
        .iter()
        .map(|logged| logged.change.at.unix_seconds())
        .collect();
    assert_eq!(left, vec![11, 10]);
}

/// Generates one `#[test]` function per contract test.
#[macro_export]
macro_rules! change_log_contract_tests {
    ($factory:expr) => {
        $crate::change_log_contract_tests!(@tests $factory;
            changes_roundtrip, changes_are_filtered, old_changes_are_pruned);
    };
    (@tests $factory:expr; $($test:ident),+) => {
        $(
            #[test]
            fn $test() {
                let log = ($factory)();
                $crate::contract::change_log::$test(&log);
            }
        )+
    };
}

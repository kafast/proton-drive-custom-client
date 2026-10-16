//! Contract tests for [`DriveRepo`] and [`ContentCache`].
//!
//! ```ignore
//! protonsync_testkit::drive_repo_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
//! protonsync_testkit::content_cache_contract_tests!(protonsync_testkit::InMemoryContentCache::new);
//! ```

use std::io::Read;

use protonsync_application::ports::{CacheKey, CacheRecord, ContentCache, DriveRepo};
use protonsync_domain::cache::{BLOCK_BYTES, BlockMap, ByteRange};
use protonsync_domain::node::{NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{PinRules, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

fn path(value: &str) -> RelativePath {
    RelativePath::parse(value).unwrap()
}

fn sorted(mut records: Vec<CacheRecord>) -> Vec<CacheRecord> {
    records.sort_by_key(|record| record.key);
    records
}

/// A partially downloaded file from Proton.
fn downloaded(key: u64) -> CacheRecord {
    let mut blocks = BlockMap::empty(3 * BLOCK_BYTES + 5);
    blocks.mark_present(ByteRange {
        offset: BLOCK_BYTES,
        length: BLOCK_BYTES,
    });
    CacheRecord {
        key: CacheKey(key),
        node: Some(NodeId::new(format!("node-{key}"))),
        path: path("Folder/File.txt"),
        revision: Some(RevisionId::new("rev-1")),
        blocks,
        last_access: Timestamp::from_unix_seconds(1_000),
        modified: Timestamp::from_unix_seconds(900),
        changed: false,
    }
}

/// A file created locally and not uploaded yet.
fn created_locally(key: u64) -> CacheRecord {
    CacheRecord {
        key: CacheKey(key),
        node: None,
        path: path("New.txt"),
        revision: None,
        blocks: BlockMap::complete(12),
        last_access: Timestamp::from_unix_seconds(2_000),
        modified: Timestamp::from_unix_seconds(2_000),
        changed: true,
    }
}

/// Entries come back unchanged, separated per drive; saving replaces,
/// removing is idempotent.
pub fn cache_records_roundtrip(repo: &impl DriveRepo) {
    let drive = SyncPairId::new("drive");
    let other = SyncPairId::new("other");
    repo.save_cache_record(&drive, &downloaded(1)).unwrap();
    repo.save_cache_record(&drive, &created_locally(2)).unwrap();
    repo.save_cache_record(&other, &downloaded(1)).unwrap();

    assert_eq!(
        sorted(repo.cache_records(&drive).unwrap()),
        vec![downloaded(1), created_locally(2)]
    );
    assert_eq!(repo.cache_records(&other).unwrap(), vec![downloaded(1)]);
    assert_eq!(
        repo.cache_records(&SyncPairId::new("empty")).unwrap(),
        Vec::new()
    );

    let mut uploaded = created_locally(2);
    uploaded.node = Some(NodeId::new("uploaded"));
    uploaded.revision = Some(RevisionId::new("rev-9"));
    uploaded.changed = false;
    repo.save_cache_record(&drive, &uploaded).unwrap();
    repo.remove_cache_record(&drive, CacheKey(1)).unwrap();
    repo.remove_cache_record(&drive, CacheKey(77)).unwrap();

    assert_eq!(repo.cache_records(&drive).unwrap(), vec![uploaded]);
    assert_eq!(repo.cache_records(&other).unwrap(), vec![downloaded(1)]);
}

/// Pin rules are set and removed per path, separated per drive.
pub fn pin_rules_roundtrip(repo: &impl DriveRepo) {
    let drive = SyncPairId::new("drive");
    let other = SyncPairId::new("other");
    repo.save_pin_rule(&drive, &path("Photos"), Some(PinState::Pinned))
        .unwrap();
    repo.save_pin_rule(&drive, &path("Photos/Raw"), Some(PinState::Unpinned))
        .unwrap();
    repo.save_pin_rule(&drive, &path("Gone"), Some(PinState::Pinned))
        .unwrap();
    repo.save_pin_rule(&other, &path("Photos"), Some(PinState::Unpinned))
        .unwrap();
    // Setting again replaces; removing an unknown rule is not an error.
    repo.save_pin_rule(&drive, &path("Gone"), None).unwrap();
    repo.save_pin_rule(&drive, &path("Never"), None).unwrap();
    repo.save_pin_rule(&other, &path("Photos"), Some(PinState::Pinned))
        .unwrap();

    let mut expected = PinRules::new();
    expected.set(path("Photos"), PinState::Pinned);
    expected.set(path("Photos/Raw"), PinState::Unpinned);
    assert_eq!(repo.pin_rules(&drive).unwrap(), expected);

    let mut expected_other = PinRules::new();
    expected_other.set(path("Photos"), PinState::Pinned);
    assert_eq!(repo.pin_rules(&other).unwrap(), expected_other);
    assert_eq!(
        repo.pin_rules(&SyncPairId::new("empty")).unwrap(),
        PinRules::new()
    );
}

/// Generates one `#[test]` function per contract test of [`DriveRepo`].
#[macro_export]
macro_rules! drive_repo_contract_tests {
    ($factory:expr) => {
        $crate::drive_repo_contract_tests!(@tests $factory; cache_records_roundtrip, pin_rules_roundtrip);
    };
    (@tests $factory:expr; $($test:ident),+) => {
        $(
            #[test]
            fn $test() {
                let repo = ($factory)();
                $crate::contract::drive::$test(&repo);
            }
        )+
    };
}

fn read_all(cache: &impl ContentCache, key: CacheKey) -> Vec<u8> {
    let mut content = Vec::new();
    cache.open(key).unwrap().read_to_end(&mut content).unwrap();
    content
}

/// Written data comes back; gaps are zeros, reads past the end return less.
pub fn cache_writes_and_reads(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"Hello").unwrap();
    cache.write(key, 10, b"Worl").unwrap();
    cache.write(key, 1, b"A").unwrap();

    assert_eq!(cache.read(key, 0, 5).unwrap(), b"HAllo");
    assert_eq!(cache.read(key, 4, 8).unwrap(), b"o\0\0\0\0\0Wo");
    assert_eq!(cache.read(key, 12, 100).unwrap(), b"rl");
    assert_eq!(cache.read(key, 50, 10).unwrap(), b"");
    assert_eq!(read_all(cache, key), b"HAllo\0\0\0\0\0Worl");
    // Contents are separated per key; missing content counts as empty.
    assert_eq!(cache.read(CacheKey(2), 0, 10).unwrap(), b"");
}

/// The length can be shortened and extended with zeros.
pub fn cache_sets_length(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"abcdef").unwrap();
    cache.set_len(key, 3).unwrap();
    assert_eq!(read_all(cache, key), b"abc");
    cache.set_len(key, 5).unwrap();
    assert_eq!(read_all(cache, key), b"abc\0\0");
    // Even a file that does not exist yet can be given a length.
    cache.set_len(CacheKey(2), 2).unwrap();
    assert_eq!(read_all(cache, CacheKey(2)), b"\0\0");
}

/// Removing deletes the content and is idempotent; opening afterwards fails.
pub fn cache_removes_content(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"gone").unwrap();
    cache.write(CacheKey(2), 0, b"stays").unwrap();
    cache.remove(key).unwrap();
    cache.remove(key).unwrap();

    assert!(cache.open(key).is_err());
    assert_eq!(cache.read(key, 0, 10).unwrap(), b"");
    assert_eq!(read_all(cache, CacheKey(2)), b"stays");
}

/// Generates one `#[test]` function per contract test of [`ContentCache`].
///
/// With `deref:` the factory returns a wrapper (e.g. with a temporary
/// directory) that points to the cache via `Deref`.
#[macro_export]
macro_rules! content_cache_contract_tests {
    (deref: $factory:expr) => {
        $crate::content_cache_contract_tests!(@tests $factory, deref;
            cache_writes_and_reads, cache_sets_length, cache_removes_content);
    };
    ($factory:expr) => {
        $crate::content_cache_contract_tests!(@tests $factory, direct;
            cache_writes_and_reads, cache_sets_length, cache_removes_content);
    };
    (@tests $factory:expr, $mode:ident; $($test:ident),+) => {
        $(
            #[test]
            fn $test() {
                let cache = ($factory)();
                $crate::contract::drive::$test($crate::content_cache_contract_tests!(@access $mode, cache));
            }
        )+
    };
    (@access deref, $cache:ident) => { &*$cache };
    (@access direct, $cache:ident) => { &$cache };
}

//! Contract-Tests für [`DriveRepo`] und [`ContentCache`].
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

/// Eine teilweise geladene Datei aus Proton.
fn downloaded(key: u64) -> CacheRecord {
    let mut blocks = BlockMap::empty(3 * BLOCK_BYTES + 5);
    blocks.mark_present(ByteRange {
        offset: BLOCK_BYTES,
        length: BLOCK_BYTES,
    });
    CacheRecord {
        key: CacheKey(key),
        node: Some(NodeId::new(format!("node-{key}"))),
        path: path("Ordner/Datei.txt"),
        revision: Some(RevisionId::new("rev-1")),
        blocks,
        last_access: Timestamp::from_unix_seconds(1_000),
        modified: Timestamp::from_unix_seconds(900),
        changed: false,
    }
}

/// Eine lokal angelegte, noch nicht hochgeladene Datei.
fn created_locally(key: u64) -> CacheRecord {
    CacheRecord {
        key: CacheKey(key),
        node: None,
        path: path("Neu.txt"),
        revision: None,
        blocks: BlockMap::complete(12),
        last_access: Timestamp::from_unix_seconds(2_000),
        modified: Timestamp::from_unix_seconds(2_000),
        changed: true,
    }
}

/// Einträge kommen unverändert zurück, je Laufwerk getrennt; Speichern ersetzt,
/// Entfernen ist idempotent.
pub fn cache_records_roundtrip(repo: &impl DriveRepo) {
    let drive = SyncPairId::new("laufwerk");
    let other = SyncPairId::new("anderes");
    repo.save_cache_record(&drive, &downloaded(1)).unwrap();
    repo.save_cache_record(&drive, &created_locally(2)).unwrap();
    repo.save_cache_record(&other, &downloaded(1)).unwrap();

    assert_eq!(
        sorted(repo.cache_records(&drive).unwrap()),
        vec![downloaded(1), created_locally(2)]
    );
    assert_eq!(repo.cache_records(&other).unwrap(), vec![downloaded(1)]);
    assert_eq!(
        repo.cache_records(&SyncPairId::new("leer")).unwrap(),
        Vec::new()
    );

    let mut uploaded = created_locally(2);
    uploaded.node = Some(NodeId::new("hochgeladen"));
    uploaded.revision = Some(RevisionId::new("rev-9"));
    uploaded.changed = false;
    repo.save_cache_record(&drive, &uploaded).unwrap();
    repo.remove_cache_record(&drive, CacheKey(1)).unwrap();
    repo.remove_cache_record(&drive, CacheKey(77)).unwrap();

    assert_eq!(repo.cache_records(&drive).unwrap(), vec![uploaded]);
    assert_eq!(repo.cache_records(&other).unwrap(), vec![downloaded(1)]);
}

/// Anheft-Regeln werden je Pfad gesetzt und entfernt, je Laufwerk getrennt.
pub fn pin_rules_roundtrip(repo: &impl DriveRepo) {
    let drive = SyncPairId::new("laufwerk");
    let other = SyncPairId::new("anderes");
    repo.save_pin_rule(&drive, &path("Fotos"), Some(PinState::Pinned))
        .unwrap();
    repo.save_pin_rule(&drive, &path("Fotos/Roh"), Some(PinState::Unpinned))
        .unwrap();
    repo.save_pin_rule(&drive, &path("Weg"), Some(PinState::Pinned))
        .unwrap();
    repo.save_pin_rule(&other, &path("Fotos"), Some(PinState::Unpinned))
        .unwrap();
    // Neu setzen ersetzt, Entfernen einer unbekannten Regel ist kein Fehler.
    repo.save_pin_rule(&drive, &path("Weg"), None).unwrap();
    repo.save_pin_rule(&drive, &path("Nie"), None).unwrap();
    repo.save_pin_rule(&other, &path("Fotos"), Some(PinState::Pinned))
        .unwrap();

    let mut expected = PinRules::new();
    expected.set(path("Fotos"), PinState::Pinned);
    expected.set(path("Fotos/Roh"), PinState::Unpinned);
    assert_eq!(repo.pin_rules(&drive).unwrap(), expected);

    let mut expected_other = PinRules::new();
    expected_other.set(path("Fotos"), PinState::Pinned);
    assert_eq!(repo.pin_rules(&other).unwrap(), expected_other);
    assert_eq!(
        repo.pin_rules(&SyncPairId::new("leer")).unwrap(),
        PinRules::new()
    );
}

/// Erzeugt für jeden Contract-Test von [`DriveRepo`] eine `#[test]`-Funktion.
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

/// Geschriebenes kommt zurück; Lücken sind Nullen, hinter dem Ende kommt weniger.
pub fn cache_writes_and_reads(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"Hallo").unwrap();
    cache.write(key, 10, b"Welt").unwrap();
    cache.write(key, 1, b"A").unwrap();

    assert_eq!(cache.read(key, 0, 5).unwrap(), b"HAllo");
    assert_eq!(cache.read(key, 4, 8).unwrap(), b"o\0\0\0\0\0We");
    assert_eq!(cache.read(key, 12, 100).unwrap(), b"lt");
    assert_eq!(cache.read(key, 50, 10).unwrap(), b"");
    assert_eq!(read_all(cache, key), b"HAllo\0\0\0\0\0Welt");
    // Inhalte sind je Schlüssel getrennt; ein fehlender Inhalt gilt als leer.
    assert_eq!(cache.read(CacheKey(2), 0, 10).unwrap(), b"");
}

/// Die Länge lässt sich kürzen und mit Nullen verlängern.
pub fn cache_sets_length(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"abcdef").unwrap();
    cache.set_len(key, 3).unwrap();
    assert_eq!(read_all(cache, key), b"abc");
    cache.set_len(key, 5).unwrap();
    assert_eq!(read_all(cache, key), b"abc\0\0");
    // Auch eine noch fehlende Datei lässt sich auf eine Länge bringen.
    cache.set_len(CacheKey(2), 2).unwrap();
    assert_eq!(read_all(cache, CacheKey(2)), b"\0\0");
}

/// Entfernen löscht den Inhalt, ist idempotent; Öffnen danach schlägt fehl.
pub fn cache_removes_content(cache: &impl ContentCache) {
    let key = CacheKey(1);
    cache.write(key, 0, b"weg").unwrap();
    cache.write(CacheKey(2), 0, b"bleibt").unwrap();
    cache.remove(key).unwrap();
    cache.remove(key).unwrap();

    assert!(cache.open(key).is_err());
    assert_eq!(cache.read(key, 0, 10).unwrap(), b"");
    assert_eq!(read_all(cache, CacheKey(2)), b"bleibt");
}

/// Erzeugt für jeden Contract-Test von [`ContentCache`] eine `#[test]`-Funktion.
///
/// Mit `deref:` liefert die Fabrik einen Wrapper (z. B. samt temporärem
/// Verzeichnis), der per `Deref` auf den Zwischenspeicher zeigt.
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

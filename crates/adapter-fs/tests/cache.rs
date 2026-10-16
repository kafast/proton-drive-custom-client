//! Der Zwischenspeicher auf der Platte besteht die Contract-Tests für `ContentCache`.

// Hilfsfunktionen in Testdateien dürfen wie Tests abbrechen.
#![allow(clippy::unwrap_used)]

use protonsync_adapter_fs::FsContentCache;
use protonsync_application::ports::{CacheKey, ContentCache};

/// Hält das temporäre Verzeichnis am Leben, solange der Zwischenspeicher benutzt wird.
struct OnDisk {
    cache: FsContentCache,
    _dir: tempfile::TempDir,
}

impl std::ops::Deref for OnDisk {
    type Target = FsContentCache;
    fn deref(&self) -> &FsContentCache {
        &self.cache
    }
}

fn open() -> OnDisk {
    let dir = tempfile::tempdir().unwrap();
    let cache = FsContentCache::open(dir.path().join("cache").join("laufwerk")).unwrap();
    OnDisk { cache, _dir: dir }
}

mod contract {
    use super::open;

    protonsync_testkit::content_cache_contract_tests!(deref: open);
}

#[cfg(unix)]
#[test]
fn cache_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let cache = open();
    cache.write(CacheKey(1), 0, b"geheim").unwrap();
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(cache.dir()), 0o700);
    assert_eq!(mode(&cache.dir().join("1")), 0o600);
}

/// Eine Lücke mitten in der Datei belegt keinen Platz (dünn belegte Datei).
#[cfg(unix)]
#[test]
fn gaps_stay_sparse() {
    use std::os::unix::fs::MetadataExt;
    const GAP: u64 = 64 * 1024 * 1024;
    // `blocks()` zählt in Einheiten von 512 Bytes.
    const BYTES_PER_BLOCK: u64 = 512;

    let cache = open();
    cache.write(CacheKey(1), GAP, b"Ende").unwrap();
    let metadata = std::fs::metadata(cache.dir().join("1")).unwrap();
    assert_eq!(metadata.len(), GAP + 4);
    assert!(metadata.blocks() * BYTES_PER_BLOCK < GAP / 2);
}

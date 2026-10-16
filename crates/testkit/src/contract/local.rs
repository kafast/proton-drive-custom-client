//! Contract-Tests für [`LocalStorage`].
//!
//! ```ignore
//! protonsync_testkit::local_storage_contract_tests!(protonsync_testkit::FakeLocal::new);
//! ```
//!
//! Die Fabrik muss einen **leeren** Wurzelordner liefern.

use std::io::{Read, Write};

use protonsync_application::ports::{LocalError, LocalKind, LocalStorage};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;

const MODIFIED: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);

fn path(value: &str) -> RelativePath {
    RelativePath::parse(value).unwrap()
}

/// Schreibt eine Datei über `stage` und `commit`.
fn write<L: LocalStorage>(local: &L, at: &str, content: &[u8]) {
    let mut staging = local.stage(&path(at)).unwrap();
    staging.write_all(content).unwrap();
    local.commit(staging, &path(at), MODIFIED).unwrap();
}

fn read<L: LocalStorage>(local: &L, at: &str) -> Vec<u8> {
    let mut content = Vec::new();
    local
        .open(&path(at))
        .unwrap()
        .read_to_end(&mut content)
        .unwrap();
    content
}

fn scanned_paths<L: LocalStorage>(local: &L) -> Vec<String> {
    local
        .scan()
        .unwrap()
        .into_iter()
        .map(|entry| entry.path.as_str().to_owned())
        .collect()
}

/// Ein neuer Wurzelordner ist leer.
pub fn starts_empty<L: LocalStorage>(local: &L) {
    assert_eq!(scanned_paths(local), Vec::<String>::new());
}

/// Geschriebene Dateien sind lesbar und tragen Größe und Änderungszeit.
pub fn write_then_read<L: LocalStorage>(local: &L) {
    write(local, "a.txt", b"hallo");
    assert_eq!(read(local, "a.txt"), b"hallo");
    let entry = local.entry(&path("a.txt")).unwrap().unwrap();
    let LocalKind::File(fingerprint) = entry.kind else {
        unreachable!("Datei erwartet")
    };
    assert_eq!(fingerprint.size, 5);
    assert_eq!(fingerprint.modified, MODIFIED);
}

/// Erst `commit` macht eine Datei sichtbar; vorhandene werden ersetzt.
pub fn staged_file_is_invisible_until_commit<L: LocalStorage>(local: &L) {
    write(local, "a.txt", b"alt");
    let mut staging = local.stage(&path("a.txt")).unwrap();
    staging.write_all(b"neu").unwrap();
    assert_eq!(read(local, "a.txt"), b"alt");
    assert_eq!(scanned_paths(local), ["a.txt"]);
    local.commit(staging, &path("a.txt"), MODIFIED).unwrap();
    assert_eq!(read(local, "a.txt"), b"neu");
}

/// Ordner werden angelegt, erscheinen im Scan vor ihrem Inhalt und sind idempotent.
pub fn folders_are_scanned_in_path_order<L: LocalStorage>(local: &L) {
    local.create_folder(&path("b")).unwrap();
    local.create_folder(&path("b")).unwrap();
    write(local, "b/x.txt", b"x");
    write(local, "a.txt", b"a");
    assert_eq!(scanned_paths(local), ["a.txt", "b", "b/x.txt"]);
    let folder = local.entry(&path("b")).unwrap().unwrap();
    assert_eq!(folder.kind, LocalKind::Folder);
}

/// Ohne Elternordner kann nichts angelegt werden.
pub fn missing_parent_is_reported<L: LocalStorage>(local: &L) {
    assert!(matches!(
        local.stage(&path("fehlt/a.txt")),
        Err(LocalError::NotFound(_))
    ));
    assert!(matches!(
        local.create_folder(&path("fehlt/b")),
        Err(LocalError::NotFound(_))
    ));
}

/// Unbekannte Pfade ergeben `None` bzw. `NotFound`.
pub fn unknown_paths<L: LocalStorage>(local: &L) {
    assert_eq!(local.entry(&path("nix")).unwrap(), None);
    assert!(matches!(
        local.open(&path("nix")),
        Err(LocalError::NotFound(_))
    ));
}

/// Die Prüfsumme ist SHA-1 als Hex.
pub fn content_hash_is_sha1_hex<L: LocalStorage>(local: &L) {
    write(local, "abc", b"abc");
    assert_eq!(
        local.content_hash(&path("abc")).unwrap(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

/// Umbenennen verschiebt auch den Inhalt von Ordnern und überschreibt nichts.
pub fn rename_moves_trees<L: LocalStorage>(local: &L) {
    local.create_folder(&path("alt")).unwrap();
    write(local, "alt/f", b"1");
    write(local, "belegt", b"2");
    local.rename(&path("alt"), &path("neu")).unwrap();
    assert_eq!(scanned_paths(local), ["belegt", "neu", "neu/f"]);
    assert!(matches!(
        local.rename(&path("neu"), &path("belegt")),
        Err(LocalError::AlreadyExists(_))
    ));
}

/// Papierkorb: verschieben, wiederherstellen, endgültig löschen.
pub fn trash_restore_and_purge<L: LocalStorage>(local: &L) {
    local.create_folder(&path("ordner")).unwrap();
    write(local, "ordner/f", b"inhalt");
    let key = local.move_to_trash(&path("ordner")).unwrap();
    assert_eq!(scanned_paths(local), Vec::<String>::new());

    local.restore_from_trash(&key, &path("zurueck")).unwrap();
    assert_eq!(read(local, "zurueck/f"), b"inhalt");

    let again = local.move_to_trash(&path("zurueck")).unwrap();
    local.purge_trash(&again).unwrap();
    assert!(local.restore_from_trash(&again, &path("zurueck")).is_err());
}

/// Erzeugt für jeden Contract-Test eine `#[test]`-Funktion.
#[macro_export]
macro_rules! local_storage_contract_tests {
    ($factory:expr) => {
        $crate::local_storage_contract_tests!(@tests $factory;
            starts_empty,
            write_then_read,
            staged_file_is_invisible_until_commit,
            folders_are_scanned_in_path_order,
            missing_parent_is_reported,
            unknown_paths,
            content_hash_is_sha1_hex,
            rename_moves_trees,
            trash_restore_and_purge,
        );
    };
    (@tests $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                let local = ($factory)();
                $crate::contract::local::$test(&local);
            }
        )+
    };
}

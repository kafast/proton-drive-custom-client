//! Contract tests for [`LocalStorage`].
//!
//! ```ignore
//! protonsync_testkit::local_storage_contract_tests!(protonsync_testkit::FakeLocal::new);
//! ```
//!
//! The factory must return an **empty** root folder.

use std::io::{Read, Write};

use protonsync_application::ports::{LocalError, LocalKind, LocalStorage};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;

const MODIFIED: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);

fn path(value: &str) -> RelativePath {
    RelativePath::parse(value).unwrap()
}

/// Writes a file through `stage` and `commit`.
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

/// A new root folder is empty.
pub fn starts_empty<L: LocalStorage>(local: &L) {
    assert_eq!(scanned_paths(local), Vec::<String>::new());
}

/// Written files are readable and carry size and modification time.
pub fn write_then_read<L: LocalStorage>(local: &L) {
    write(local, "a.txt", b"hello");
    assert_eq!(read(local, "a.txt"), b"hello");
    let entry = local.entry(&path("a.txt")).unwrap().unwrap();
    let LocalKind::File(fingerprint) = entry.kind else {
        unreachable!("expected a file")
    };
    assert_eq!(fingerprint.size, 5);
    assert_eq!(fingerprint.modified, MODIFIED);
}

/// Only `commit` makes a file visible; existing ones are replaced.
pub fn staged_file_is_invisible_until_commit<L: LocalStorage>(local: &L) {
    write(local, "a.txt", b"old");
    let mut staging = local.stage(&path("a.txt")).unwrap();
    staging.write_all(b"new").unwrap();
    assert_eq!(read(local, "a.txt"), b"old");
    assert_eq!(scanned_paths(local), ["a.txt"]);
    local.commit(staging, &path("a.txt"), MODIFIED).unwrap();
    assert_eq!(read(local, "a.txt"), b"new");
}

/// Folders are created, appear in the scan before their content and are idempotent.
pub fn folders_are_scanned_in_path_order<L: LocalStorage>(local: &L) {
    local.create_folder(&path("b")).unwrap();
    local.create_folder(&path("b")).unwrap();
    write(local, "b/x.txt", b"x");
    write(local, "a.txt", b"a");
    assert_eq!(scanned_paths(local), ["a.txt", "b", "b/x.txt"]);
    let folder = local.entry(&path("b")).unwrap().unwrap();
    assert_eq!(folder.kind, LocalKind::Folder);
}

/// Nothing can be created without a parent folder.
pub fn missing_parent_is_reported<L: LocalStorage>(local: &L) {
    assert!(matches!(
        local.stage(&path("missing/a.txt")),
        Err(LocalError::NotFound(_))
    ));
    assert!(matches!(
        local.create_folder(&path("missing/b")),
        Err(LocalError::NotFound(_))
    ));
}

/// Unknown paths yield `None` or `NotFound`.
pub fn unknown_paths<L: LocalStorage>(local: &L) {
    assert_eq!(local.entry(&path("nothing")).unwrap(), None);
    assert!(matches!(
        local.open(&path("nothing")),
        Err(LocalError::NotFound(_))
    ));
}

/// The checksum is SHA-1 as hex.
pub fn content_hash_is_sha1_hex<L: LocalStorage>(local: &L) {
    write(local, "abc", b"abc");
    assert_eq!(
        local.content_hash(&path("abc")).unwrap(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

/// Renaming also moves the content of folders and overwrites nothing.
pub fn rename_moves_trees<L: LocalStorage>(local: &L) {
    local.create_folder(&path("old")).unwrap();
    write(local, "old/f", b"1");
    write(local, "taken", b"2");
    local.rename(&path("old"), &path("new")).unwrap();
    assert_eq!(scanned_paths(local), ["new", "new/f", "taken"]);
    assert!(matches!(
        local.rename(&path("new"), &path("taken")),
        Err(LocalError::AlreadyExists(_))
    ));
}

/// Trash: move, restore, delete permanently.
pub fn trash_restore_and_purge<L: LocalStorage>(local: &L) {
    local.create_folder(&path("folder")).unwrap();
    write(local, "folder/f", b"content");
    let key = local.move_to_trash(&path("folder")).unwrap();
    assert_eq!(scanned_paths(local), Vec::<String>::new());

    local.restore_from_trash(&key, &path("restored")).unwrap();
    assert_eq!(read(local, "restored/f"), b"content");

    let again = local.move_to_trash(&path("restored")).unwrap();
    local.purge_trash(&again).unwrap();
    assert!(local.restore_from_trash(&again, &path("restored")).is_err());
}

/// Generates one `#[test]` function per contract test.
///
/// Variant `deref: factory` for factories that return a wrapper (e.g. adapter
/// plus temporary directory) that points to the implementation via `Deref`.
#[macro_export]
macro_rules! local_storage_contract_tests {
    (deref: $factory:expr) => {
        $crate::local_storage_contract_tests!(@tests $factory, deref;
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
    ($factory:expr) => {
        $crate::local_storage_contract_tests!(@tests $factory, direct;
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
    (@tests $factory:expr, $mode:ident; $($test:ident),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                let local = ($factory)();
                $crate::contract::local::$test($crate::local_storage_contract_tests!(@access $mode, local));
            }
        )+
    };
    (@access direct, $value:ident) => { &$value };
    (@access deref, $value:ident) => { &*$value };
}

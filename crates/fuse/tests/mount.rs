//! Echter FUSE-Mount über den Fakes (kein Proton-Konto nötig).
//!
//! Braucht `/dev/fuse` und `fusermount3` (Paket `fuse3`), deshalb `#[ignore]`:
//!
//! ```sh
//! cargo test -p protonsync-fuse --test mount -- --ignored
//! ```

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::io::{Read, Seek, SeekFrom};

use common::Setup;
use protonsync_application::ports::RemoteStorage;

#[test]
#[ignore = "braucht /dev/fuse und fusermount3"]
fn files_can_be_used_through_the_mount() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let setup = runtime.block_on(async {
        let s = Setup::new().await;
        let docs = s.remote_folder("Docs").await;
        s.remote_file(&docs, "vorhanden.txt", b"aus Proton").await;
        s
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = protonsync_fuse::mount(
        std::sync::Arc::clone(&setup.drive),
        dir.path(),
        runtime.handle().clone(),
    )
    .unwrap();
    let root = dir.path();

    // Lesen (auch ab einer Position) lädt bei Bedarf.
    let existing = root.join("Docs/vorhanden.txt");
    assert_eq!(fs::read_to_string(&existing).unwrap(), "aus Proton");
    let mut file = fs::File::open(&existing).unwrap();
    file.seek(SeekFrom::Start(4)).unwrap();
    let mut rest = String::new();
    file.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "Proton");
    drop(file);

    // Schreiben, Umbenennen, Ordner anlegen, Löschen.
    fs::write(root.join("Docs/neu.txt"), "lokal geschrieben").unwrap();
    fs::create_dir(root.join("Archiv")).unwrap();
    fs::rename(root.join("Docs/neu.txt"), root.join("Archiv/neu.txt")).unwrap();
    fs::remove_file(&existing).unwrap();
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["Archiv", "Docs"]);
    assert!(fs::metadata(root.join("Archiv")).unwrap().is_dir());

    handle.unmount().unwrap();

    // In Proton ist alles angekommen.
    runtime.block_on(async {
        let names = setup.remote_names(&setup.root).await;
        assert_eq!(names, ["Archiv", "Docs"]);
        let archive = setup
            .remote
            .list_children(&setup.remote.list_children(&setup.root.id).await.unwrap()[0].id)
            .await
            .unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(
            setup.remote_content(&archive[0]).await,
            b"lokal geschrieben"
        );
    });
}

#[test]
#[ignore = "braucht /dev/fuse und fusermount3"]
fn large_files_are_read_and_overwritten_correctly() {
    use protonsync_domain::cache::BLOCK_BYTES;

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let length = usize::try_from(2 * BLOCK_BYTES + 12_345).unwrap();
    let content: Vec<u8> = (0..length)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let (setup, file) = runtime.block_on(async {
        let s = Setup::new().await;
        let file = s.remote_file(&s.root, "gross.bin", &content).await;
        (s, file)
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = protonsync_fuse::mount(
        std::sync::Arc::clone(&setup.drive),
        dir.path(),
        runtime.handle().clone(),
    )
    .unwrap();
    let path = dir.path().join("gross.bin");

    assert_eq!(fs::metadata(&path).unwrap().len(), length as u64);
    assert_eq!(fs::read(&path).unwrap(), content);
    // Überschreiben (öffnet mit O_TRUNC) und ein Editor-Speichern über eine Hilfsdatei.
    fs::write(&path, b"ersetzt").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"ersetzt");
    let temp = dir.path().join(".gross.bin.swp");
    fs::write(&temp, b"gespeichert").unwrap();
    fs::rename(&temp, &path).unwrap();

    handle.unmount().unwrap();
    runtime.block_on(async {
        assert_eq!(setup.remote_names(&setup.root).await, ["gross.bin"]);
        let current = setup.remote.list_children(&setup.root.id).await.unwrap();
        assert_eq!(setup.remote_content(&current[0]).await, b"gespeichert");
        assert_eq!(
            current[0].id, file.id,
            "neue Fassung derselben Datei, der Verlauf in Proton bleibt"
        );
    });
}

#[test]
#[ignore = "braucht /dev/fuse und fusermount3"]
fn a_missing_mountpoint_is_created() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let setup = runtime.block_on(Setup::new());
    let dir = tempfile::tempdir().unwrap();
    let mountpoint = dir.path().join("neu/Laufwerk");
    let handle = protonsync_fuse::mount(
        std::sync::Arc::clone(&setup.drive),
        &mountpoint,
        runtime.handle().clone(),
    )
    .unwrap();
    assert_eq!(handle.mountpoint(), mountpoint);
    assert_eq!(fs::read_dir(&mountpoint).unwrap().count(), 0);
    handle.unmount().unwrap();
}

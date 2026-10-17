//! Real FUSE mount on top of the fakes (no Proton account needed).
//!
//! Needs `/dev/fuse` and `fusermount3` (package `fuse3`), hence `#[ignore]`:
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
#[ignore = "needs /dev/fuse and fusermount3"]
fn files_can_be_used_through_the_mount() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let setup = runtime.block_on(async {
        let s = Setup::new().await;
        let docs = s.remote_folder("Docs").await;
        s.remote_file(&docs, "existing.txt", b"from Proton").await;
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

    // Reading (also from a position) downloads on demand.
    let existing = root.join("Docs/existing.txt");
    assert_eq!(fs::read_to_string(&existing).unwrap(), "from Proton");
    let mut file = fs::File::open(&existing).unwrap();
    file.seek(SeekFrom::Start(5)).unwrap();
    let mut rest = String::new();
    file.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "Proton");
    drop(file);

    // Writing, renaming, creating a folder, deleting.
    fs::write(root.join("Docs/new.txt"), "written locally").unwrap();
    fs::create_dir(root.join("Archive")).unwrap();
    fs::rename(root.join("Docs/new.txt"), root.join("Archive/new.txt")).unwrap();
    fs::remove_file(&existing).unwrap();
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["Archive", "Docs"]);
    assert!(fs::metadata(root.join("Archive")).unwrap().is_dir());

    handle.unmount().unwrap();

    // Everything has arrived in Proton.
    runtime.block_on(async {
        let names = setup.remote_names(&setup.root).await;
        assert_eq!(names, ["Archive", "Docs"]);
        let archive = setup
            .remote
            .list_children(&setup.remote.list_children(&setup.root.id).await.unwrap()[0].id)
            .await
            .unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(setup.remote_content(&archive[0]).await, b"written locally");
    });
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn large_files_are_read_and_overwritten_correctly() {
    use protonsync_domain::cache::BLOCK_BYTES;

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let length = usize::try_from(2 * BLOCK_BYTES + 12_345).unwrap();
    let content: Vec<u8> = (0..length)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let (setup, file) = runtime.block_on(async {
        let s = Setup::new().await;
        let file = s.remote_file(&s.root, "large.bin", &content).await;
        (s, file)
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = protonsync_fuse::mount(
        std::sync::Arc::clone(&setup.drive),
        dir.path(),
        runtime.handle().clone(),
    )
    .unwrap();
    let path = dir.path().join("large.bin");

    assert_eq!(fs::metadata(&path).unwrap().len(), length as u64);
    assert_eq!(fs::read(&path).unwrap(), content);
    // Overwriting (opens with O_TRUNC) and an editor save via a temporary file.
    fs::write(&path, b"replaced").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"replaced");
    let temp = dir.path().join(".large.bin.swp");
    fs::write(&temp, b"saved").unwrap();
    fs::rename(&temp, &path).unwrap();

    handle.unmount().unwrap();
    runtime.block_on(async {
        assert_eq!(setup.remote_names(&setup.root).await, ["large.bin"]);
        let current = setup.remote.list_children(&setup.root.id).await.unwrap();
        assert_eq!(setup.remote_content(&current[0]).await, b"saved");
        assert_eq!(
            current[0].id, file.id,
            "new revision of the same file, the history in Proton is kept"
        );
    });
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_missing_mountpoint_is_created() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let setup = runtime.block_on(Setup::new());
    let dir = tempfile::tempdir().unwrap();
    let mountpoint = dir.path().join("new/Drive");
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

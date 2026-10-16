//! Übersetzung der Kernel-Anfragen gegen die Fakes (ohne eingehängtes Dateisystem).

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, UNIX_EPOCH};

use common::{NOW, OWNER, Setup};
use fuser::{Errno, FileType, INodeNo, OpenFlags, RenameFlags};
use protonsync_domain::cache::BLOCK_BYTES;

const READ_ONLY: OpenFlags = OpenFlags(libc::O_RDONLY);
const READ_WRITE: OpenFlags = OpenFlags(libc::O_RDWR);
const TRUNCATE: OpenFlags = OpenFlags(libc::O_WRONLY | libc::O_TRUNC);

fn os(text: &str) -> &OsStr {
    OsStr::new(text)
}

#[tokio::test]
async fn attributes_look_like_ordinary_files() {
    let s = Setup::new().await;
    s.remote_file(&s.root, "a.txt", b"hallo").await;
    let ops = s.ops();

    let file = ops.lookup(INodeNo::ROOT, os("a.txt")).await.unwrap();
    assert_eq!(file.kind, FileType::RegularFile);
    assert_eq!(file.size, 5);
    assert_eq!(file.blocks, 1);
    assert_eq!(file.perm, 0o644);
    assert_eq!((file.uid, file.gid), (OWNER.uid, OWNER.gid));
    let seconds = u64::try_from(NOW.unix_seconds()).unwrap();
    assert_eq!(file.mtime, UNIX_EPOCH + Duration::from_secs(seconds));

    let root = ops.getattr(INodeNo::ROOT).unwrap();
    assert_eq!(root.kind, FileType::Directory);
    assert_eq!(root.nlink, 2);
    assert_eq!(ops.getattr(INodeNo(9_999)), Err(Errno::ENOENT));
}

#[tokio::test]
async fn names_are_checked() {
    let s = Setup::new().await;
    let ops = s.ops();
    let not_utf8 = OsStr::from_bytes(b"\xff\xfe");
    assert_eq!(
        ops.lookup(INodeNo::ROOT, not_utf8).await,
        Err(Errno::EINVAL)
    );
    assert_eq!(ops.mkdir(INodeNo::ROOT, os("")).await, Err(Errno::EINVAL));
    assert_eq!(
        ops.lookup(INodeNo::ROOT, os("fehlt")).await,
        Err(Errno::ENOENT)
    );
}

#[tokio::test]
async fn listing_contains_dot_entries_and_children() {
    let s = Setup::new().await;
    s.remote_folder("Docs").await;
    s.remote_file(&s.root, "b.txt", b"x").await;
    let ops = s.ops();

    let listing = ops.list(INodeNo::ROOT).await.unwrap();
    let names: Vec<(&str, FileType)> = listing
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind))
        .collect();
    assert_eq!(
        names,
        [
            (".", FileType::Directory),
            ("..", FileType::Directory),
            ("Docs", FileType::Directory),
            ("b.txt", FileType::RegularFile),
        ]
    );
    let file = listing[3].ino;
    assert_eq!(ops.list(file).await, Err(Errno::ENOTDIR));
}

#[tokio::test]
async fn open_with_truncate_does_not_download_the_old_content() {
    let s = Setup::new().await;
    let content = vec![7_u8; usize::try_from(2 * BLOCK_BYTES).unwrap()];
    let file = s.remote_file(&s.root, "gross.bin", &content).await;
    let ops = s.ops();
    let ino = ops
        .lookup(INodeNo::ROOT, os("gross.bin"))
        .await
        .unwrap()
        .ino;

    ops.open(ino, TRUNCATE).await.unwrap();
    assert_eq!(ops.write(ino, 0, b"klein").unwrap(), 5);
    ops.release(ino).await;

    assert_eq!(s.remote.range_bytes(), 0);
    assert_eq!(s.remote_content(&file).await, b"klein");
}

#[tokio::test]
async fn reading_and_writing_go_through_the_drive() {
    let s = Setup::new().await;
    let file = s.remote_file(&s.root, "notiz.txt", b"eins").await;
    let ops = s.ops();
    let ino = ops
        .lookup(INodeNo::ROOT, os("notiz.txt"))
        .await
        .unwrap()
        .ino;

    ops.open(ino, READ_ONLY).await.unwrap();
    assert_eq!(ops.read(ino, 1, 10).await.unwrap(), b"ins");
    ops.release(ino).await;

    ops.open(ino, READ_WRITE).await.unwrap();
    ops.write(ino, 4, b" zwei").unwrap();
    assert_eq!(ops.setattr(ino, None).await.unwrap().size, 9);
    ops.release(ino).await;
    assert_eq!(s.remote_content(&file).await, b"eins zwei");

    let attr = ops.setattr(ino, Some(2)).await.unwrap();
    assert_eq!(attr.size, 2);
    assert_eq!(ops.read(INodeNo::ROOT, 0, 1).await, Err(Errno::EISDIR));
}

#[tokio::test]
async fn files_and_folders_can_be_created_and_removed() {
    let s = Setup::new().await;
    let ops = s.ops();

    let folder = ops.mkdir(INodeNo::ROOT, os("Neu")).await.unwrap();
    assert_eq!(folder.kind, FileType::Directory);
    assert_eq!(
        ops.mkdir(INodeNo::ROOT, os("Neu")).await,
        Err(Errno::EEXIST)
    );
    let file = ops.create(folder.ino, os("a.txt")).await.unwrap();
    ops.write(file.ino, 0, b"inhalt").unwrap();
    ops.release(file.ino).await;
    assert_eq!(
        ops.rmdir(INodeNo::ROOT, os("Neu")).await,
        Err(Errno::ENOTEMPTY)
    );
    assert_eq!(
        ops.unlink(INodeNo::ROOT, os("Neu")).await,
        Err(Errno::EISDIR)
    );

    ops.unlink(folder.ino, os("a.txt")).await.unwrap();
    assert_eq!(
        ops.lookup(folder.ino, os("a.txt")).await,
        Err(Errno::ENOENT)
    );
    ops.rmdir(INodeNo::ROOT, os("Neu")).await.unwrap();
    assert_eq!(s.remote_names(&s.root).await, Vec::<String>::new());
}

#[tokio::test]
async fn rename_respects_flags() {
    let s = Setup::new().await;
    s.remote_file(&s.root, "a.txt", b"a").await;
    s.remote_file(&s.root, "b.txt", b"b").await;
    let ops = s.ops();
    let root = INodeNo::ROOT;

    assert_eq!(
        ops.rename(
            root,
            os("a.txt"),
            root,
            os("b.txt"),
            RenameFlags::RENAME_NOREPLACE
        )
        .await,
        Err(Errno::EEXIST)
    );
    assert_eq!(
        ops.rename(
            root,
            os("a.txt"),
            root,
            os("b.txt"),
            RenameFlags::RENAME_EXCHANGE
        )
        .await,
        Err(Errno::EINVAL)
    );
    ops.rename(
        root,
        os("a.txt"),
        root,
        os("c.txt"),
        RenameFlags::RENAME_NOREPLACE,
    )
    .await
    .unwrap();
    // Ohne Flags ersetzt Umbenennen das Ziel (wie bei `mv`).
    ops.rename(root, os("c.txt"), root, os("b.txt"), RenameFlags::empty())
        .await
        .unwrap();
    assert_eq!(s.remote_names(&s.root).await, ["b.txt"]);
}

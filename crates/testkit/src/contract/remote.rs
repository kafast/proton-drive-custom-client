//! Contract tests for [`RemoteStorage`]: the behavior that every implementation
//! (fake, Proton adapter, maybe the official SDK later) must show.
//!
//! Include it in a test crate with a factory function that returns an **empty**
//! storage (for live tests: an empty test folder):
//!
//! ```ignore
//! protonsync_testkit::remote_storage_contract_tests!(protonsync_testkit::FakeRemote::new);
//! ```
//!
//! The calling crate needs `tokio` as a `dev-dependency`.

use protonsync_application::ports::{RemoteError, RemoteEvent, RemoteStorage};
use protonsync_domain::node::{Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::time::Timestamp;

const MODIFIED: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);

fn name(value: &str) -> NodeName {
    NodeName::new(value).unwrap()
}

fn revision_of(node: &Node) -> RevisionId {
    match &node.kind {
        NodeKind::File { revision, .. } => revision.clone(),
        NodeKind::Folder => unreachable!("expected a file"),
    }
}

/// Uploads `content` as a new file.
async fn upload<R: RemoteStorage>(
    remote: &R,
    parent: &Node,
    file_name: &str,
    content: &[u8],
) -> Result<Node, RemoteError> {
    remote
        .upload_file(
            &parent.id,
            &name(file_name),
            content,
            content.len() as u64,
            MODIFIED,
        )
        .await
}

/// Downloads the whole content of a file.
async fn download<R: RemoteStorage>(remote: &R, file: &NodeId) -> Result<Vec<u8>, RemoteError> {
    let mut content = Vec::new();
    let written = remote.download_to(file, &mut content).await?;
    assert_eq!(written, content.len() as u64);
    Ok(content)
}

async fn names_in<R: RemoteStorage>(remote: &R, folder: &Node) -> Vec<String> {
    remote
        .list_children(&folder.id)
        .await
        .unwrap()
        .into_iter()
        .map(|node| node.name.to_string())
        .collect()
}

/// The root is a folder without a parent and initially empty.
pub async fn root_is_an_empty_folder<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    assert!(root.is_folder());
    assert_eq!(root.parent, None);
    assert_eq!(names_in(remote, &root).await, Vec::<String>::new());
}

/// Created folders appear sorted in the parent folder.
pub async fn created_folders_are_listed_by_name<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let folder = remote.create_folder(&root.id, &name("b")).await.unwrap();
    remote.create_folder(&root.id, &name("a")).await.unwrap();
    assert_eq!(folder.parent.as_ref(), Some(&root.id));
    assert_eq!(names_in(remote, &root).await, ["a", "b"]);
}

/// A name may occur only once in a folder.
pub async fn duplicate_names_are_rejected<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    remote.create_folder(&root.id, &name("x")).await.unwrap();
    let result = upload(remote, &root, "x", &[1]).await;
    assert_eq!(result, Err(RemoteError::AlreadyExists(name("x"))));
}

/// Uploaded content comes back unchanged; the size is correct.
pub async fn upload_then_download_roundtrips<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let content = b"Hello Proton".to_vec();
    let file = upload(remote, &root, "hello.txt", &content).await.unwrap();
    assert_eq!(file.fingerprint().unwrap().size, content.len() as u64);
    assert_eq!(download(remote, &file.id).await.unwrap(), content);
    assert_eq!(remote.node(&file.id).await.unwrap(), file);
}

/// A new revision replaces the content and gets a new revision ID.
pub async fn new_revision_replaces_content<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let first = upload(remote, &root, "f", &[1]).await.unwrap();
    let second = remote
        .upload_revision(&first.id, &[2u8, 2][..], 2, MODIFIED)
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_ne!(revision_of(&first), revision_of(&second));
    assert_eq!(download(remote, &first.id).await.unwrap(), vec![2, 2]);
}

/// Range downloads return exactly the range and are cut off at the end of the file.
pub async fn range_download_returns_requested_bytes<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let file = upload(remote, &root, "r", b"0123456789").await.unwrap();
    assert_eq!(remote.download_range(&file.id, 2, 3).await.unwrap(), b"234");
    assert_eq!(
        remote.download_range(&file.id, 8, 100).await.unwrap(),
        b"89"
    );
    assert_eq!(remote.download_range(&file.id, 10, 1).await.unwrap(), b"");
    assert_eq!(
        remote.download_range(&file.id, 11, 1).await,
        Err(RemoteError::InvalidRange {
            offset: 11,
            size: 10
        })
    );
}

/// Folders and files are not mixed up.
pub async fn files_and_folders_are_distinguished<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let file = upload(remote, &root, "f", &[]).await.unwrap();
    assert_eq!(
        download(remote, &root.id).await,
        Err(RemoteError::NotAFile(root.id.clone()))
    );
    assert_eq!(
        remote.create_folder(&file.id, &name("x")).await,
        Err(RemoteError::NotAFolder(file.id.clone()))
    );
}

/// Renaming changes the name, but not to one that is taken.
pub async fn rename_changes_name<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let file = upload(remote, &root, "old", &[]).await.unwrap();
    remote
        .create_folder(&root.id, &name("taken"))
        .await
        .unwrap();
    let renamed = remote.rename(&file.id, &name("new")).await.unwrap();
    assert_eq!(renamed.name, name("new"));
    assert_eq!(
        remote.rename(&file.id, &name("taken")).await,
        Err(RemoteError::AlreadyExists(name("taken")))
    );
}

/// Moving changes the parent folder; a folder never into itself.
pub async fn move_changes_parent<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let outer = remote
        .create_folder(&root.id, &name("outer"))
        .await
        .unwrap();
    let inner = remote
        .create_folder(&outer.id, &name("inner"))
        .await
        .unwrap();
    let file = upload(remote, &root, "f", &[]).await.unwrap();

    let moved = remote.move_node(&file.id, &inner.id).await.unwrap();
    assert_eq!(moved.parent.as_ref(), Some(&inner.id));
    assert_eq!(names_in(remote, &inner).await, ["f"]);
    assert!(matches!(
        remote.move_node(&outer.id, &inner.id).await,
        Err(RemoteError::InvalidMove(_))
    ));
}

/// Trashed nodes disappear from the listing (with their subtree) and can be restored.
pub async fn trash_and_restore<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let folder = remote
        .create_folder(&root.id, &name("folder"))
        .await
        .unwrap();
    let file = upload(remote, &folder, "f", &[7]).await.unwrap();

    remote.trash(&folder.id).await.unwrap();
    assert_eq!(names_in(remote, &root).await, Vec::<String>::new());
    assert_eq!(
        remote.node(&file.id).await,
        Err(RemoteError::NotFound(file.id.clone()))
    );

    remote.restore(&folder.id).await.unwrap();
    assert_eq!(names_in(remote, &root).await, ["folder"]);
    assert_eq!(download(remote, &file.id).await.unwrap(), vec![7]);
}

/// Permanent deletion works only from the trash and removes the subtree.
pub async fn delete_permanently_only_from_trash<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let folder = remote.create_folder(&root.id, &name("gone")).await.unwrap();
    let file = upload(remote, &folder, "f", &[1]).await.unwrap();
    assert_eq!(
        remote.delete_permanently(&folder.id).await,
        Err(RemoteError::NotFound(folder.id.clone()))
    );
    remote.trash(&folder.id).await.unwrap();
    remote.delete_permanently(&folder.id).await.unwrap();
    assert_eq!(
        remote.restore(&folder.id).await,
        Err(RemoteError::NotFound(folder.id.clone()))
    );
    assert_eq!(
        remote.node(&file.id).await,
        Err(RemoteError::NotFound(file.id.clone()))
    );
}

/// Events start at the current cursor and are delivered only once.
///
/// Proton does not distinguish between "created" and "updated"; so for the new
/// file it is only required that it is reported at all.
pub async fn events_report_changes_once<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let (initial, cursor) = remote.events_since(None).await.unwrap();
    assert_eq!(initial, Vec::new());

    let file = upload(remote, &root, "e", &[]).await.unwrap();
    remote.trash(&file.id).await.unwrap();
    let (events, cursor) = remote.events_since(Some(&cursor)).await.unwrap();
    assert!(
        events.contains(&RemoteEvent::Created(file.id.clone()))
            || events.contains(&RemoteEvent::Updated(file.id.clone()))
    );
    assert!(events.contains(&RemoteEvent::Trashed(file.id.clone())));

    let (again, _) = remote.events_since(Some(&cursor)).await.unwrap();
    assert_eq!(again, Vec::new());
}

/// If the size read differs from the announced one, nothing is created.
pub async fn upload_with_wrong_size_is_rejected<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let result = remote
        .upload_file(&root.id, &name("broken"), &b"abc"[..], 5, MODIFIED)
        .await;
    assert!(matches!(result, Err(RemoteError::Transfer(_))));
    assert_eq!(names_in(remote, &root).await, Vec::<String>::new());
}

/// Unknown IDs yield `NotFound`.
pub async fn unknown_ids_are_not_found<R: RemoteStorage>(remote: &R) {
    let unknown = NodeId::new("does-not-exist");
    assert_eq!(
        remote.node(&unknown).await,
        Err(RemoteError::NotFound(unknown.clone()))
    );
}

/// Generates one `#[tokio::test]` function per contract test.
///
/// `$factory` is called anew for every test so the tests are independent.
///
/// With `live: $factory` the factory is asynchronous (e.g. login and a fresh
/// test folder at Proton) and the tests are `#[ignore]`, so they only run with
/// `cargo test -- --ignored`.
#[macro_export]
macro_rules! remote_storage_contract_tests {
    (live: $factory:expr) => {
        $crate::remote_storage_contract_tests!(@all @live $factory);
    };
    (@all @$mode:ident $factory:expr) => {
        $crate::remote_storage_contract_tests!(@tests @$mode $factory;
            root_is_an_empty_folder,
            created_folders_are_listed_by_name,
            duplicate_names_are_rejected,
            upload_then_download_roundtrips,
            new_revision_replaces_content,
            range_download_returns_requested_bytes,
            files_and_folders_are_distinguished,
            rename_changes_name,
            move_changes_parent,
            trash_and_restore,
            delete_permanently_only_from_trash,
            events_report_changes_once,
            upload_with_wrong_size_is_rejected,
            unknown_ids_are_not_found,
        );
    };
    (@tests @plain $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[tokio::test]
            async fn $test() {
                let remote = ($factory)();
                $crate::contract::remote::$test(&remote).await;
            }
        )+
    };
    (@tests @live $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "live test: needs a real Proton account (see test file)"]
            async fn $test() {
                let remote = ($factory)(stringify!($test)).await;
                $crate::contract::remote::$test(&remote).await;
            }
        )+
    };    // Comes last: otherwise an `expr` that does not match would be a hard error.
    ($factory:expr) => {
        $crate::remote_storage_contract_tests!(@all @plain $factory);
    };
}

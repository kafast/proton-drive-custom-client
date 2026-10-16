//! Contract-Tests für [`RemoteStorage`]: das Verhalten, das jede Implementierung
//! (Fake, Proton-Adapter, später evtl. das offizielle SDK) zeigen muss.
//!
//! Einbinden in einem Test-Crate mit einer Fabrikfunktion, die einen **leeren**
//! Speicher liefert (für Live-Tests: einen leeren Testordner):
//!
//! ```ignore
//! protonsync_testkit::remote_storage_contract_tests!(protonsync_testkit::FakeRemote::new);
//! ```
//!
//! Das aufrufende Crate braucht `tokio` als `dev-dependency`.

// Dies ist Test-Code: Ein fehlgeschlagenes `unwrap` soll den Test abbrechen.
#![allow(clippy::unwrap_used, clippy::missing_panics_doc)]

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
        NodeKind::Folder => unreachable!("Datei erwartet"),
    }
}

/// Lädt `content` als neue Datei hoch.
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

/// Lädt den ganzen Inhalt einer Datei.
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

/// Die Wurzel ist ein Ordner ohne Elternknoten und anfangs leer.
pub async fn root_is_an_empty_folder<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    assert!(root.is_folder());
    assert_eq!(root.parent, None);
    assert_eq!(names_in(remote, &root).await, Vec::<String>::new());
}

/// Angelegte Ordner erscheinen sortiert im Elternordner.
pub async fn created_folders_are_listed_by_name<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let folder = remote.create_folder(&root.id, &name("b")).await.unwrap();
    remote.create_folder(&root.id, &name("a")).await.unwrap();
    assert_eq!(folder.parent.as_ref(), Some(&root.id));
    assert_eq!(names_in(remote, &root).await, ["a", "b"]);
}

/// Ein Name darf in einem Ordner nur einmal vorkommen.
pub async fn duplicate_names_are_rejected<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    remote.create_folder(&root.id, &name("x")).await.unwrap();
    let result = upload(remote, &root, "x", &[1]).await;
    assert_eq!(result, Err(RemoteError::AlreadyExists(name("x"))));
}

/// Hochgeladener Inhalt kommt unverändert zurück; die Größe stimmt.
pub async fn upload_then_download_roundtrips<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let content = b"Hallo Proton".to_vec();
    let file = upload(remote, &root, "hallo.txt", &content).await.unwrap();
    assert_eq!(file.fingerprint().unwrap().size, content.len() as u64);
    assert_eq!(download(remote, &file.id).await.unwrap(), content);
    assert_eq!(remote.node(&file.id).await.unwrap(), file);
}

/// Eine neue Revision ersetzt den Inhalt und bekommt eine neue Revisionskennung.
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

/// Teil-Downloads liefern genau den Bereich und kürzen am Dateiende.
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

/// Ordner und Dateien werden nicht verwechselt.
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

/// Umbenennen ändert den Namen, aber nicht auf einen belegten.
pub async fn rename_changes_name<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let file = upload(remote, &root, "alt", &[]).await.unwrap();
    remote
        .create_folder(&root.id, &name("belegt"))
        .await
        .unwrap();
    let renamed = remote.rename(&file.id, &name("neu")).await.unwrap();
    assert_eq!(renamed.name, name("neu"));
    assert_eq!(
        remote.rename(&file.id, &name("belegt")).await,
        Err(RemoteError::AlreadyExists(name("belegt")))
    );
}

/// Verschieben ändert den Elternordner; ein Ordner nie in sich selbst.
pub async fn move_changes_parent<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let outer = remote
        .create_folder(&root.id, &name("aussen"))
        .await
        .unwrap();
    let inner = remote
        .create_folder(&outer.id, &name("innen"))
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

/// Gelöschtes verschwindet aus der Liste (mit Unterbaum) und lässt sich wiederherstellen.
pub async fn trash_and_restore<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let folder = remote
        .create_folder(&root.id, &name("ordner"))
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
    assert_eq!(names_in(remote, &root).await, ["ordner"]);
    assert_eq!(download(remote, &file.id).await.unwrap(), vec![7]);
}

/// Ereignisse beginnen am aktuellen Cursor und werden nur einmal geliefert.
pub async fn events_report_changes_once<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let (initial, cursor) = remote.events_since(None).await.unwrap();
    assert_eq!(initial, Vec::new());

    let file = upload(remote, &root, "e", &[]).await.unwrap();
    remote.trash(&file.id).await.unwrap();
    let (events, cursor) = remote.events_since(Some(&cursor)).await.unwrap();
    assert!(events.contains(&RemoteEvent::Created(file.id.clone())));
    assert!(events.contains(&RemoteEvent::Trashed(file.id.clone())));

    let (again, _) = remote.events_since(Some(&cursor)).await.unwrap();
    assert_eq!(again, Vec::new());
}

/// Weicht die gelesene Größe von der angekündigten ab, wird nichts angelegt.
pub async fn upload_with_wrong_size_is_rejected<R: RemoteStorage>(remote: &R) {
    let root = remote.root_folder().await.unwrap();
    let result = remote
        .upload_file(&root.id, &name("kaputt"), &b"abc"[..], 5, MODIFIED)
        .await;
    assert!(matches!(result, Err(RemoteError::Transfer(_))));
    assert_eq!(names_in(remote, &root).await, Vec::<String>::new());
}

/// Unbekannte Kennungen ergeben `NotFound`.
pub async fn unknown_ids_are_not_found<R: RemoteStorage>(remote: &R) {
    let unknown = NodeId::new("gibt-es-nicht");
    assert_eq!(
        remote.node(&unknown).await,
        Err(RemoteError::NotFound(unknown.clone()))
    );
}

/// Erzeugt für jeden Contract-Test eine `#[tokio::test]`-Funktion.
///
/// `$factory` wird für jeden Test neu aufgerufen, damit die Tests unabhängig sind.
#[macro_export]
macro_rules! remote_storage_contract_tests {
    ($factory:expr) => {
        $crate::remote_storage_contract_tests!(@tests $factory;
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
            events_report_changes_once,
            upload_with_wrong_size_is_rejected,
            unknown_ids_are_not_found,
        );
    };
    (@tests $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[tokio::test]
            async fn $test() {
                let remote = ($factory)();
                $crate::contract::$test(&remote).await;
            }
        )+
    };
}

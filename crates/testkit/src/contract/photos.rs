//! Contract tests for [`PhotoLibrary`]: the behavior that every implementation
//! must show.
//!
//! The factory function returns an **empty** library. These tests deliberately
//! do not run against a real account: photos have no folders, so a test would
//! write into the real timeline (ADR 0012).
//!
//! ```ignore
//! protonsync_testkit::photo_library_contract_tests!(protonsync_testkit::FakePhotoLibrary::new);
//! ```

use protonsync_application::ports::{NewPhoto, PhotoLibrary, RemoteError, RemoteEvent};
use protonsync_domain::node::{NodeId, NodeName};
use protonsync_domain::time::Timestamp;

use crate::hash::sha1_hex;

const CAPTURED: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);

fn new_photo(name: &str, content: &[u8]) -> NewPhoto {
    NewPhoto {
        name: NodeName::new(name).unwrap(),
        media_type: "image/jpeg".into(),
        size: content.len() as u64,
        captured: CAPTURED,
    }
}

async fn download<P: PhotoLibrary>(library: &P, id: &NodeId) -> Result<Vec<u8>, RemoteError> {
    let mut content = Vec::new();
    let written = library.download_to(id, &mut content).await?;
    assert_eq!(written, content.len() as u64);
    Ok(content)
}

async fn ids<P: PhotoLibrary>(library: &P) -> Vec<NodeId> {
    library
        .photos()
        .await
        .unwrap()
        .into_iter()
        .map(|photo| photo.id)
        .collect()
}

/// An empty library lists nothing.
pub async fn empty_library_lists_nothing<P: PhotoLibrary>(library: &P) {
    assert_eq!(ids(library).await, Vec::<NodeId>::new());
}

/// An uploaded photo appears with its attributes and can be downloaded.
pub async fn upload_then_list_and_download<P: PhotoLibrary>(library: &P) {
    let content = b"JPEG content";
    let uploaded = library
        .upload(&new_photo("IMG_1.jpg", content), &content[..])
        .await
        .unwrap();
    assert_eq!(uploaded.name.as_str(), "IMG_1.jpg");
    assert_eq!(uploaded.captured, CAPTURED);
    assert_eq!(uploaded.size, content.len() as u64);
    assert_eq!(
        uploaded.content_hash.as_deref(),
        Some(sha1_hex(content).as_str())
    );
    assert!(!uploaded.is_related());

    assert_eq!(library.photo(&uploaded.id).await.unwrap(), uploaded);
    assert_eq!(library.photos().await.unwrap(), vec![uploaded.clone()]);
    assert_eq!(download(library, &uploaded.id).await.unwrap(), content);
}

/// If the announced size is wrong, nothing is created.
pub async fn upload_with_wrong_size_is_rejected<P: PhotoLibrary>(library: &P) {
    let mut photo = new_photo("broken.jpg", b"abc");
    photo.size = 10;
    assert!(matches!(
        library.upload(&photo, &b"abc"[..]).await,
        Err(RemoteError::Transfer(_))
    ));
    assert_eq!(ids(library).await, Vec::<NodeId>::new());
}

/// Photos in the trash disappear from the timeline and come back.
pub async fn trash_and_restore<P: PhotoLibrary>(library: &P) {
    let photo = library
        .upload(&new_photo("a.jpg", b"a"), &b"a"[..])
        .await
        .unwrap();
    library.trash(&photo.id).await.unwrap();
    assert_eq!(ids(library).await, Vec::<NodeId>::new());
    assert!(matches!(
        library.photo(&photo.id).await,
        Err(RemoteError::NotFound(_))
    ));
    library.restore(&photo.id).await.unwrap();
    assert_eq!(ids(library).await, vec![photo.id.clone()]);
    assert!(matches!(
        library.restore(&photo.id).await,
        Err(RemoteError::NotFound(_))
    ));
}

/// Permanent deletion works only from the trash.
pub async fn delete_permanently_only_from_trash<P: PhotoLibrary>(library: &P) {
    let photo = library
        .upload(&new_photo("b.jpg", b"b"), &b"b"[..])
        .await
        .unwrap();
    assert!(matches!(
        library.delete_permanently(&photo.id).await,
        Err(RemoteError::NotFound(_))
    ));
    library.trash(&photo.id).await.unwrap();
    library.delete_permanently(&photo.id).await.unwrap();
    assert!(matches!(
        library.restore(&photo.id).await,
        Err(RemoteError::NotFound(_))
    ));
}

/// Events report new and trashed photos exactly once.
pub async fn events_report_changes_once<P: PhotoLibrary>(library: &P) {
    let (events, start) = library.events_since(None).await.unwrap();
    assert_eq!(events, Vec::new());
    let photo = library
        .upload(&new_photo("c.jpg", b"c"), &b"c"[..])
        .await
        .unwrap();
    library.trash(&photo.id).await.unwrap();
    let (events, next) = library.events_since(Some(&start)).await.unwrap();
    assert!(
        events.contains(&RemoteEvent::Created(photo.id.clone())),
        "{events:?}"
    );
    assert!(
        events.contains(&RemoteEvent::Trashed(photo.id.clone())),
        "{events:?}"
    );
    let (events, _) = library.events_since(Some(&next)).await.unwrap();
    assert_eq!(events, Vec::new());
}

/// Unknown IDs yield `NotFound`.
pub async fn unknown_ids_are_not_found<P: PhotoLibrary>(library: &P) {
    let unknown = NodeId::new("does-not-exist");
    assert!(matches!(
        library.photo(&unknown).await,
        Err(RemoteError::NotFound(_))
    ));
    assert!(matches!(
        download(library, &unknown).await,
        Err(RemoteError::NotFound(_))
    ));
    assert!(matches!(
        library.trash(&unknown).await,
        Err(RemoteError::NotFound(_))
    ));
}

/// Generates the contract tests for a [`PhotoLibrary`] factory.
#[macro_export]
macro_rules! photo_library_contract_tests {
    ($factory:expr) => {
        $crate::photo_library_contract_tests!(@tests $factory;
            empty_library_lists_nothing,
            upload_then_list_and_download,
            upload_with_wrong_size_is_rejected,
            trash_and_restore,
            delete_permanently_only_from_trash,
            events_report_changes_once,
            unknown_ids_are_not_found,
        );
    };
    (@tests $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[tokio::test]
            async fn $test() {
                let library = ($factory)();
                $crate::contract::photos::$test(&library).await;
            }
        )+
    };
}

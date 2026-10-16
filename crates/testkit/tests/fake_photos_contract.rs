//! The `FakePhotoLibrary` must pass the photo library contract tests.

protonsync_testkit::photo_library_contract_tests!(protonsync_testkit::FakePhotoLibrary::new);

use protonsync_application::ports::{PhotoLibrary, RemoteError, RemoteEvent};

#[tokio::test]
async fn changes_from_the_phone_produce_events() {
    let library = protonsync_testkit::FakePhotoLibrary::new();
    let (_, start) = library.events_since(None).await.unwrap();
    let photo = library.add_from_phone("IMG_1.jpg", b"one", 100);
    library.modify(&photo.id, |photo, content| {
        photo.favorite = true;
        content.extend_from_slice(b"!");
    });
    library.trash_from_phone(&photo.id);
    library.push_gap();
    let (events, _) = library.events_since(Some(&start)).await.unwrap();
    assert_eq!(
        events,
        vec![
            RemoteEvent::Created(photo.id.clone()),
            RemoteEvent::Updated(photo.id.clone()),
            RemoteEvent::Trashed(photo.id.clone()),
            RemoteEvent::Gap,
        ]
    );
    assert_eq!(library.content(&photo.id).as_deref(), Some(&b"one!"[..]));
    assert!(library.is_trashed(&photo.id));
}

#[tokio::test]
async fn injected_failure_hits_only_the_next_call() {
    let library = protonsync_testkit::FakePhotoLibrary::new();
    library.fail_next_call(RemoteError::Unavailable("network down".into()));
    assert!(matches!(
        library.photos().await,
        Err(RemoteError::Unavailable(_))
    ));
    assert!(library.photos().await.is_ok());
    assert_eq!(library.list_calls(), 1);
}

/// The shared library (`Arc`) must behave the same way.
mod shared {
    protonsync_testkit::photo_library_contract_tests!(|| std::sync::Arc::new(
        protonsync_testkit::FakePhotoLibrary::new()
    ));
}

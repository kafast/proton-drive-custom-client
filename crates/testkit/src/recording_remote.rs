use std::io::{Read, Write};
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{
    EventCursor, NewPhoto, PhotoLibrary, RemoteError, RemoteEvent, RemoteStorage,
};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;

/// A call that changes something in the remote storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteWrite {
    /// [`RemoteStorage::create_folder`].
    CreateFolder {
        /// Parent folder.
        parent: NodeId,
        /// Name of the new folder.
        name: String,
    },
    /// [`RemoteStorage::upload_file`].
    UploadFile {
        /// Parent folder.
        parent: NodeId,
        /// Name of the new file.
        name: String,
    },
    /// [`RemoteStorage::upload_revision`].
    UploadRevision(NodeId),
    /// [`RemoteStorage::rename`].
    Rename {
        /// Renamed node.
        id: NodeId,
        /// New name.
        new_name: String,
    },
    /// [`RemoteStorage::move_node`].
    Move {
        /// Moved node.
        id: NodeId,
        /// New parent folder.
        new_parent: NodeId,
    },
    /// [`RemoteStorage::trash`].
    Trash(NodeId),
    /// [`RemoteStorage::restore`].
    Restore(NodeId),
    /// [`RemoteStorage::delete_permanently`].
    DeletePermanently(NodeId),
}

/// Wrapper around any [`RemoteStorage`] that records every changing call
/// before passing it on. Reading calls are passed on unrecorded.
/// With [`RecordingRemote::fail_next_downloads`] downloads can fail on request
/// (e.g. network loss in the middle of a run).
///
/// Tests of the modes that must never change anything in Proton
/// (e.g. `download_only`, `backup_only`) hand this wrapper to the code under
/// test and check [`RecordingRemote::writes`] afterwards. Changes by "another
/// device" go to the inner storage directly and are therefore not recorded.
#[derive(Debug)]
pub struct RecordingRemote<'a, R> {
    inner: &'a R,
    writes: Mutex<Vec<RemoteWrite>>,
    /// Error for the next downloads and how many of them still fail.
    download_failure: Mutex<Option<(RemoteError, u32)>>,
}

impl<'a, R: RemoteStorage> RecordingRemote<'a, R> {
    /// Wraps `inner`; nothing is recorded yet.
    #[must_use]
    pub fn new(inner: &'a R) -> Self {
        Self {
            inner,
            writes: Mutex::new(Vec::new()),
            download_failure: Mutex::new(None),
        }
    }

    /// The next `count` downloads (whole or in parts) fail with `error`.
    pub fn fail_next_downloads(&self, error: RemoteError, count: u32) {
        *lock(&self.download_failure) = Some((error, count));
    }

    fn take_download_failure(&self) -> Result<(), RemoteError> {
        let mut failure = lock(&self.download_failure);
        match failure.take() {
            Some((error, count)) if count > 0 => {
                if count > 1 {
                    *failure = Some((error.clone(), count - 1));
                }
                Err(error)
            }
            _ => Ok(()),
        }
    }

    /// All changing calls so far, in call order.
    #[must_use]
    pub fn writes(&self) -> Vec<RemoteWrite> {
        lock(&self.writes).clone()
    }

    fn record(&self, write: RemoteWrite) {
        lock(&self.writes).push(write);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another test thread panicked; the value is still usable.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl<R: RemoteStorage> RemoteStorage for RecordingRemote<'_, R> {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        self.inner.root_folder().await
    }

    async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.inner.node(id).await
    }

    async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
        self.inner.list_children(folder).await
    }

    async fn create_folder(&self, parent: &NodeId, name: &NodeName) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::CreateFolder {
            parent: parent.clone(),
            name: name.to_string(),
        });
        self.inner.create_folder(parent, name).await
    }

    async fn upload_file<C: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: C,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::UploadFile {
            parent: parent.clone(),
            name: name.to_string(),
        });
        self.inner
            .upload_file(parent, name, content, size, modified)
            .await
    }

    async fn upload_revision<C: Read + Send>(
        &self,
        file: &NodeId,
        content: C,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::UploadRevision(file.clone()));
        self.inner
            .upload_revision(file, content, size, modified)
            .await
    }

    async fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        self.take_download_failure()?;
        self.inner.download_to(file, target).await
    }

    async fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        self.take_download_failure()?;
        self.inner.download_range(file, offset, length).await
    }

    async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::Rename {
            id: id.clone(),
            new_name: new_name.to_string(),
        });
        self.inner.rename(id, new_name).await
    }

    async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::Move {
            id: id.clone(),
            new_parent: new_parent.clone(),
        });
        self.inner.move_node(id, new_parent).await
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.record(RemoteWrite::Trash(id.clone()));
        self.inner.trash(id).await
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.record(RemoteWrite::Restore(id.clone()));
        self.inner.restore(id).await
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.record(RemoteWrite::DeletePermanently(id.clone()));
        self.inner.delete_permanently(id).await
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        self.inner.events_since(cursor).await
    }
}

/// A call that changes something in the photo library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoWrite {
    /// [`PhotoLibrary::upload`] with the name of the new photo.
    Upload(String),
    /// [`PhotoLibrary::trash`].
    Trash(NodeId),
    /// [`PhotoLibrary::restore`].
    Restore(NodeId),
    /// [`PhotoLibrary::delete_permanently`].
    DeletePermanently(NodeId),
}

/// Like [`RecordingRemote`], for a [`PhotoLibrary`]: records every changing call
/// before passing it on.
#[derive(Debug)]
pub struct RecordingPhotoLibrary<'a, P> {
    inner: &'a P,
    writes: Mutex<Vec<PhotoWrite>>,
}

impl<'a, P: PhotoLibrary> RecordingPhotoLibrary<'a, P> {
    /// Wraps `inner`; nothing is recorded yet.
    #[must_use]
    pub fn new(inner: &'a P) -> Self {
        Self {
            inner,
            writes: Mutex::new(Vec::new()),
        }
    }

    /// All changing calls so far, in call order.
    #[must_use]
    pub fn writes(&self) -> Vec<PhotoWrite> {
        lock(&self.writes).clone()
    }

    fn record(&self, write: PhotoWrite) {
        lock(&self.writes).push(write);
    }
}

impl<P: PhotoLibrary> PhotoLibrary for RecordingPhotoLibrary<'_, P> {
    async fn photos(&self) -> Result<Vec<Photo>, RemoteError> {
        self.inner.photos().await
    }

    async fn photo(&self, id: &NodeId) -> Result<Photo, RemoteError> {
        self.inner.photo(id).await
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        self.inner.events_since(cursor).await
    }

    async fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        self.inner.download_to(id, target).await
    }

    async fn upload<C: Read + Send>(
        &self,
        photo: &NewPhoto,
        content: C,
    ) -> Result<Photo, RemoteError> {
        self.record(PhotoWrite::Upload(photo.name.to_string()));
        self.inner.upload(photo, content).await
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.record(PhotoWrite::Trash(id.clone()));
        self.inner.trash(id).await
    }

    async fn restore(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.record(PhotoWrite::Restore(id.clone()));
        self.inner.restore(id).await
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.record(PhotoWrite::DeletePermanently(id.clone()));
        self.inner.delete_permanently(id).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::FakeRemote;

    #[tokio::test]
    async fn records_only_changing_calls() {
        let fake = FakeRemote::new();
        let remote = RecordingRemote::new(&fake);
        let root = remote.root_folder().await.unwrap();
        let name = NodeName::new("a.txt").unwrap();

        let file = remote
            .upload_file(
                &root.id,
                &name,
                &b"a"[..],
                1,
                Timestamp::from_unix_seconds(0),
            )
            .await
            .unwrap();
        remote.list_children(&root.id).await.unwrap();
        remote.download_to(&file.id, &mut Vec::new()).await.unwrap();
        remote.trash(&file.id).await.unwrap();

        assert_eq!(
            remote.writes(),
            [
                RemoteWrite::UploadFile {
                    parent: root.id,
                    name: "a.txt".into()
                },
                RemoteWrite::Trash(file.id)
            ]
        );
    }

    #[tokio::test]
    async fn fails_the_requested_number_of_downloads() {
        let fake = FakeRemote::new();
        let remote = RecordingRemote::new(&fake);
        let root = remote.root_folder().await.unwrap();
        let file = fake
            .upload_file(
                &root.id,
                &NodeName::new("a.txt").unwrap(),
                &b"a"[..],
                1,
                Timestamp::from_unix_seconds(0),
            )
            .await
            .unwrap();
        remote.fail_next_downloads(RemoteError::Unauthorized, 2);

        assert!(remote.download_to(&file.id, &mut Vec::new()).await.is_err());
        assert!(remote.download_range(&file.id, 0, 1).await.is_err());
        assert_eq!(remote.download_range(&file.id, 0, 1).await.unwrap(), b"a");
        assert_eq!(remote.writes(), []);
    }

    #[tokio::test]
    async fn records_only_changing_photo_calls() {
        let fake = crate::FakePhotoLibrary::new();
        let photo = fake.add_from_phone("a.jpg", b"a", 0);
        let library = RecordingPhotoLibrary::new(&fake);

        library.photos().await.unwrap();
        library
            .download_to(&photo.id, &mut Vec::new())
            .await
            .unwrap();
        library.trash(&photo.id).await.unwrap();
        library.restore(&photo.id).await.unwrap();

        assert_eq!(
            library.writes(),
            [
                PhotoWrite::Trash(photo.id.clone()),
                PhotoWrite::Restore(photo.id)
            ]
        );
    }
}

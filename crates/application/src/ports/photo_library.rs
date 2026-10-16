//! Port to the photo library (Proton Photos or a fake for tests, ADR 0012).
//!
//! Photos form a flat timeline without folders; therefore there is neither
//! renaming nor moving. The errors are the same as for the remote
//! storage ([`RemoteError`]). All implementations must pass the
//! contract tests in `protonsync-testkit`.

use std::future::Future;
use std::io::{Read, Write};

use protonsync_domain::node::{NodeId, NodeName};
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;

use super::{EventCursor, RemoteBin, RemoteError, RemoteEvent};

/// What belongs to a new photo when uploading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPhoto {
    /// File name.
    pub name: NodeName,
    /// Media type, e.g. `image/jpeg`.
    pub media_type: String,
    /// Size in bytes.
    pub size: u64,
    /// Capture time by which the timeline is sorted.
    pub captured: Timestamp,
}

/// Abstract photo library.
pub trait PhotoLibrary: Send + Sync {
    /// All photos of the timeline (without the trash), including related photos.
    /// An account without a photo area returns an empty list.
    fn photos(&self) -> impl Future<Output = Result<Vec<Photo>, RemoteError>> + Send;

    /// A single photo. [`RemoteError::NotFound`] if it is not there (any more),
    /// is in the trash or is not a photo (e.g. an album).
    fn photo(&self, id: &NodeId) -> impl Future<Output = Result<Photo, RemoteError>> + Send;

    /// Events of the library after `cursor` and the new cursor; with `None`
    /// only the current cursor as a starting point.
    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send;

    /// Writes the content to `target` and returns the number of bytes.
    fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send;

    /// Uploads a new photo.
    fn upload<R: Read + Send>(
        &self,
        photo: &NewPhoto,
        content: R,
    ) -> impl Future<Output = Result<Photo, RemoteError>> + Send;

    /// Moves a photo to the Proton trash.
    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Brings a photo back from the Proton trash.
    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Permanently deletes a photo from the trash; only for photos in the trash.
    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;
}

/// A shared library (`Arc`) is itself a library.
impl<T: PhotoLibrary + ?Sized> PhotoLibrary for std::sync::Arc<T> {
    fn photos(&self) -> impl Future<Output = Result<Vec<Photo>, RemoteError>> + Send {
        (**self).photos()
    }

    fn photo(&self, id: &NodeId) -> impl Future<Output = Result<Photo, RemoteError>> + Send {
        (**self).photo(id)
    }

    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send {
        (**self).events_since(cursor)
    }

    fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send {
        (**self).download_to(id, target)
    }

    fn upload<R: Read + Send>(
        &self,
        photo: &NewPhoto,
        content: R,
    ) -> impl Future<Output = Result<Photo, RemoteError>> + Send {
        (**self).upload(photo, content)
    }

    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send {
        (**self).trash(id)
    }

    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send {
        (**self).restore(id)
    }

    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send {
        (**self).delete_permanently(id)
    }
}

/// Makes a photo library the trash partner of the [`RemoteBin`], so that
/// the same trash use case also restores and deletes photos.
pub struct PhotoBin<'a, P>(pub &'a P);

impl<P: PhotoLibrary> RemoteBin for PhotoBin<'_, P> {
    fn restore_from_trash(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send {
        self.0.restore(id)
    }

    fn delete_from_trash(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send {
        self.0.delete_permanently(id)
    }
}

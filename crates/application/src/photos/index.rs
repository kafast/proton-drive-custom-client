//! The library in memory, kept up to date through events (ADR 0012).
//!
//! On the first run (and after a gap in the event stream) it is read
//! completely once; afterwards only changed photos are fetched individually.

use std::collections::BTreeMap;

use protonsync_domain::node::NodeId;
use protonsync_domain::photo::Photo;

use crate::ports::{EventCursor, PhotoLibrary, RemoteError, RemoteEvent};

/// What an update did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexRefresh {
    /// The library was read completely.
    Full,
    /// This many events were caught up on (at least one).
    Events(usize),
    /// Nothing has changed since last time.
    Unchanged,
}

/// All photos of the library with the event cursor up to which they are correct.
#[derive(Debug, Clone, Default)]
pub struct PhotoIndex {
    photos: BTreeMap<NodeId, Photo>,
    cursor: Option<EventCursor>,
}

impl PhotoIndex {
    /// Empty index; the first update reads completely.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` once the library has been read.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.cursor.is_some()
    }

    /// All known photos, sorted by identifier.
    pub fn photos(&self) -> impl Iterator<Item = &Photo> {
        self.photos.values()
    }

    /// Number of known photos.
    #[must_use]
    pub fn len(&self) -> usize {
        self.photos.len()
    }

    /// `true` if no photo is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.photos.is_empty()
    }

    /// A photo by identifier.
    #[must_use]
    pub fn get(&self, id: &NodeId) -> Option<&Photo> {
        self.photos.get(id)
    }

    /// Forces a complete read next time (safety net).
    pub fn invalidate(&mut self) {
        self.cursor = None;
    }

    /// Adds a photo that was just uploaded.
    pub(crate) fn insert(&mut self, photo: Photo) {
        self.photos.insert(photo.id.clone(), photo);
    }

    /// Forgets a photo (e.g. after moving it to the trash).
    pub(crate) fn remove(&mut self, id: &NodeId) {
        self.photos.remove(id);
    }

    /// Brings the index up to date.
    ///
    /// # Errors
    /// [`RemoteError`] from the library. The cursor then stays put, so that
    /// the next attempt processes the same events again.
    pub async fn refresh<P: PhotoLibrary>(
        &mut self,
        library: &P,
    ) -> Result<IndexRefresh, RemoteError> {
        let Some(cursor) = self.cursor.clone() else {
            return self.load_all(library).await;
        };
        let (events, next) = library.events_since(Some(&cursor)).await?;
        if events.is_empty() {
            self.cursor = Some(next);
            return Ok(IndexRefresh::Unchanged);
        }
        if events.contains(&RemoteEvent::Gap) {
            return self.load_all(library).await;
        }
        for event in &events {
            match event {
                RemoteEvent::Created(id) | RemoteEvent::Updated(id) => {
                    self.reload(library, id).await?;
                }
                RemoteEvent::Trashed(id) => self.remove(id),
                RemoteEvent::Gap => {}
            }
        }
        self.cursor = Some(next);
        Ok(IndexRefresh::Events(events.len()))
    }

    async fn load_all<P: PhotoLibrary>(
        &mut self,
        library: &P,
    ) -> Result<IndexRefresh, RemoteError> {
        // Fetch the cursor first, then read: whatever happens in between arrives as an
        // event once more next time and is then fetched.
        let (_, cursor) = library.events_since(None).await?;
        let photos = library.photos().await?;
        self.photos = photos
            .into_iter()
            .map(|photo| (photo.id.clone(), photo))
            .collect();
        self.cursor = Some(cursor);
        Ok(IndexRefresh::Full)
    }

    async fn reload<P: PhotoLibrary>(
        &mut self,
        library: &P,
        id: &NodeId,
    ) -> Result<(), RemoteError> {
        match library.photo(id).await {
            Ok(photo) => self.insert(photo),
            // Deleted, in the trash or not a photo (e.g. an album).
            Err(RemoteError::NotFound(_)) => self.remove(id),
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

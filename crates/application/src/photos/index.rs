//! Die Bibliothek im Speicher, aktuell gehalten über Ereignisse (ADR 0012).
//!
//! Beim ersten Lauf (und nach einer Lücke im Ereignisstrom) wird sie einmal
//! vollständig gelesen; danach werden nur noch geänderte Fotos einzeln nachgeladen.

use std::collections::BTreeMap;

use protonsync_domain::node::NodeId;
use protonsync_domain::photo::Photo;

use crate::ports::{EventCursor, PhotoLibrary, RemoteError, RemoteEvent};

/// Was eine Aktualisierung getan hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexRefresh {
    /// Die Bibliothek wurde vollständig gelesen.
    Full,
    /// So viele Ereignisse wurden nachgeholt (mindestens eins).
    Events(usize),
    /// Seit dem letzten Mal hat sich nichts geändert.
    Unchanged,
}

/// Alle Fotos der Bibliothek mit dem Ereignis-Cursor, bis zu dem sie stimmen.
#[derive(Debug, Clone, Default)]
pub struct PhotoIndex {
    photos: BTreeMap<NodeId, Photo>,
    cursor: Option<EventCursor>,
}

impl PhotoIndex {
    /// Leerer Index; die erste Aktualisierung liest vollständig.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `true`, sobald die Bibliothek einmal gelesen wurde.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.cursor.is_some()
    }

    /// Alle bekannten Fotos, nach Kennung sortiert.
    pub fn photos(&self) -> impl Iterator<Item = &Photo> {
        self.photos.values()
    }

    /// Anzahl bekannter Fotos.
    #[must_use]
    pub fn len(&self) -> usize {
        self.photos.len()
    }

    /// `true`, wenn kein Foto bekannt ist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.photos.is_empty()
    }

    /// Ein Foto nach Kennung.
    #[must_use]
    pub fn get(&self, id: &NodeId) -> Option<&Photo> {
        self.photos.get(id)
    }

    /// Erzwingt beim nächsten Mal ein vollständiges Lesen (Sicherheitsnetz).
    pub fn invalidate(&mut self) {
        self.cursor = None;
    }

    /// Nimmt ein gerade hochgeladenes Foto auf.
    pub(crate) fn insert(&mut self, photo: Photo) {
        self.photos.insert(photo.id.clone(), photo);
    }

    /// Vergisst ein Foto (z. B. nach dem Papierkorb).
    pub(crate) fn remove(&mut self, id: &NodeId) {
        self.photos.remove(id);
    }

    /// Bringt den Index auf den neuesten Stand.
    ///
    /// # Errors
    /// [`RemoteError`] von der Bibliothek. Der Cursor bleibt dann stehen, so dass
    /// der nächste Versuch dieselben Ereignisse noch einmal verarbeitet.
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
        // Erst den Cursor holen, dann lesen: Was dazwischen passiert, kommt als
        // Ereignis beim nächsten Mal noch einmal und wird dann nachgeladen.
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
            // Gelöscht, im Papierkorb oder kein Foto (z. B. ein Album).
            Err(RemoteError::NotFound(_)) => self.remove(id),
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

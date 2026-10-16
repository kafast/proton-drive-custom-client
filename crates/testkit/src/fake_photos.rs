use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{
    EventCursor, NewPhoto, PhotoLibrary, RemoteError, RemoteEvent,
};
use protonsync_domain::node::{NodeId, NodeName, RevisionId};
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;

use crate::hash::sha1_hex;

/// Ab hier zählen die Upload-Zeitpunkte im Fake (2023-11-14).
const FIRST_UPLOAD: i64 = 1_700_000_000;

/// Deterministischer In-Memory-Ersatz für Proton Photos.
///
/// Kennungen werden fortlaufend vergeben (`p1`, `p2`, … und `pr1`, `pr2`, …).
/// Mit [`FakePhotoLibrary::add_from_phone`] und [`FakePhotoLibrary::trash_from_phone`]
/// lassen sich Änderungen anderer Geräte nachstellen.
#[derive(Debug, Default)]
pub struct FakePhotoLibrary {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    photos: BTreeMap<NodeId, Entry>,
    events: Vec<RemoteEvent>,
    next_number: u64,
    pending_failure: Option<RemoteError>,
    list_calls: u64,
    /// Änderungen ohne Ereignis (simuliert verlorene Ereignisse).
    silent: bool,
}

#[derive(Debug, Clone)]
struct Entry {
    photo: Photo,
    content: Vec<u8>,
    trashed: bool,
}

impl State {
    fn record(&mut self, event: RemoteEvent) {
        if !self.silent {
            self.events.push(event);
        }
    }

    fn next(&mut self) -> u64 {
        self.next_number += 1;
        self.next_number
    }

    fn create(&mut self, new: &NewPhoto, content: Vec<u8>) -> Photo {
        let number = self.next();
        let photo = Photo {
            id: NodeId::new(format!("p{number}")),
            name: new.name.clone(),
            captured: new.captured,
            uploaded: Timestamp::from_unix_seconds(
                FIRST_UPLOAD + i64::try_from(number).unwrap_or(0),
            ),
            revision: RevisionId::new(format!("pr{number}")),
            size: content.len() as u64,
            content_hash: Some(sha1_hex(&content)),
            media_type: new.media_type.clone(),
            main_photo: None,
            favorite: false,
            albums: 0,
        };
        self.record(RemoteEvent::Created(photo.id.clone()));
        self.photos.insert(
            photo.id.clone(),
            Entry {
                photo: photo.clone(),
                content,
                trashed: false,
            },
        );
        photo
    }

    fn entry(&mut self, id: &NodeId) -> Result<&mut Entry, RemoteError> {
        self.photos
            .get_mut(id)
            .ok_or_else(|| RemoteError::NotFound(id.clone()))
    }

    fn active(&mut self, id: &NodeId) -> Result<&mut Entry, RemoteError> {
        match self.entry(id)? {
            entry if !entry.trashed => Ok(entry),
            _ => Err(RemoteError::NotFound(id.clone())),
        }
    }
}

impl FakePhotoLibrary {
    /// Leere Bibliothek.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ein anderes Gerät (das Handy) lädt ein Foto hoch.
    ///
    /// # Panics
    /// Bei einem ungültigen Namen (Test-Fehler).
    pub fn add_from_phone(&self, name: &str, content: &[u8], captured: i64) -> Photo {
        let new = NewPhoto {
            name: NodeName::new(name).unwrap_or_else(|e| unreachable!("ungültiger Testname: {e}")),
            media_type: "image/jpeg".into(),
            size: content.len() as u64,
            captured: Timestamp::from_unix_seconds(captured),
        };
        self.lock().create(&new, content.to_vec())
    }

    /// Ein anderes Gerät legt ein Foto in den Papierkorb.
    pub fn trash_from_phone(&self, id: &NodeId) {
        let mut state = self.lock();
        if let Ok(entry) = state.active(id) {
            entry.trashed = true;
            state.record(RemoteEvent::Trashed(id.clone()));
        }
    }

    /// Ändert ein Foto mit `change` (z. B. Favorit setzen, Revision ersetzen)
    /// und meldet es als geändert.
    pub fn modify(&self, id: &NodeId, change: impl FnOnce(&mut Photo, &mut Vec<u8>)) {
        let mut state = self.lock();
        if let Ok(entry) = state.active(id) {
            change(&mut entry.photo, &mut entry.content);
            entry.photo.size = entry.content.len() as u64;
            entry.photo.content_hash = Some(sha1_hex(&entry.content));
            state.record(RemoteEvent::Updated(id.clone()));
        }
    }

    /// Solange `true`, erzeugen Änderungen keine Ereignisse (verlorene Ereignisse).
    pub fn silence_events(&self, silent: bool) {
        self.lock().silent = silent;
    }

    /// Meldet eine Lücke im Ereignisstrom.
    pub fn push_gap(&self) {
        self.lock().events.push(RemoteEvent::Gap);
    }

    /// Der nächste Aufruf (egal welcher) schlägt mit `error` fehl.
    pub fn fail_next_call(&self, error: RemoteError) {
        self.lock().pending_failure = Some(error);
    }

    /// Bisherige vollständige Auflistungen (Maß für die Last bei Proton).
    #[must_use]
    pub fn list_calls(&self) -> u64 {
        self.lock().list_calls
    }

    /// Inhalt eines Fotos, auch im Papierkorb; `None`, wenn es endgültig weg ist.
    #[must_use]
    pub fn content(&self, id: &NodeId) -> Option<Vec<u8>> {
        self.lock()
            .photos
            .get(id)
            .map(|entry| entry.content.clone())
    }

    /// `true`, wenn das Foto im Papierkorb liegt.
    #[must_use]
    pub fn is_trashed(&self, id: &NodeId) -> bool {
        self.lock()
            .photos
            .get(id)
            .is_some_and(|entry| entry.trashed)
    }

    /// Kennungen aller Fotos außerhalb des Papierkorbs.
    #[must_use]
    pub fn active_ids(&self) -> Vec<NodeId> {
        self.lock()
            .photos
            .values()
            .filter(|entry| !entry.trashed)
            .map(|entry| entry.photo.id.clone())
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Ein vergifteter Mutex heißt: Ein anderer Test-Thread ist abgestürzt;
        // der Zustand selbst ist dann trotzdem brauchbar.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut State) -> Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        let mut state = self.lock();
        if let Some(error) = state.pending_failure.take() {
            return Err(error);
        }
        operation(&mut state)
    }
}

// Der Fake antwortet sofort; `async` bleibt, damit er wie der echte Adapter aussieht.
#[allow(clippy::unused_async_trait_impl)]
impl PhotoLibrary for FakePhotoLibrary {
    async fn photos(&self) -> Result<Vec<Photo>, RemoteError> {
        self.with_state(|state| {
            state.list_calls += 1;
            Ok(state
                .photos
                .values()
                .filter(|entry| !entry.trashed)
                .map(|entry| entry.photo.clone())
                .collect())
        })
    }

    async fn photo(&self, id: &NodeId) -> Result<Photo, RemoteError> {
        self.with_state(|state| Ok(state.active(id)?.photo.clone()))
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        self.with_state(|state| {
            let end = state.events.len();
            let start = match cursor {
                None => end,
                Some(cursor) => cursor
                    .as_str()
                    .parse::<usize>()
                    .ok()
                    .filter(|start| *start <= end)
                    .ok_or_else(|| RemoteError::Rejected("unbekannter Cursor".into()))?,
            };
            Ok((
                state.events[start..].to_vec(),
                EventCursor::new(end.to_string()),
            ))
        })
    }

    async fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        let content = self.with_state(|state| Ok(state.active(id)?.content.clone()))?;
        target
            .write_all(&content)
            .map_err(|e| RemoteError::Transfer(e.to_string()))?;
        Ok(content.len() as u64)
    }

    async fn upload<R: Read + Send>(
        &self,
        photo: &NewPhoto,
        mut content: R,
    ) -> Result<Photo, RemoteError> {
        let mut bytes = Vec::new();
        content
            .read_to_end(&mut bytes)
            .map_err(|e| RemoteError::Transfer(e.to_string()))?;
        if bytes.len() as u64 != photo.size {
            return Err(RemoteError::Transfer(format!(
                "{} Bytes erwartet, {} gelesen",
                photo.size,
                bytes.len()
            )));
        }
        self.with_state(|state| Ok(state.create(photo, bytes)))
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.with_state(|state| {
            state.active(id)?.trashed = true;
            state.record(RemoteEvent::Trashed(id.clone()));
            Ok(())
        })
    }

    async fn restore(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.with_state(|state| {
            let entry = state.entry(id)?;
            if !entry.trashed {
                return Err(RemoteError::NotFound(id.clone()));
            }
            entry.trashed = false;
            state.record(RemoteEvent::Created(id.clone()));
            Ok(())
        })
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.with_state(|state| {
            if !state.entry(id)?.trashed {
                return Err(RemoteError::NotFound(id.clone()));
            }
            state.photos.remove(id);
            Ok(())
        })
    }
}

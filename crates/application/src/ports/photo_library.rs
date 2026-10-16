//! Port zur Fotobibliothek (Proton Photos oder ein Fake für Tests, ADR 0012).
//!
//! Fotos bilden eine flache Zeitleiste ohne Ordner; deshalb gibt es weder
//! Umbenennen noch Verschieben. Fehler sind dieselben wie beim entfernten
//! Speicher ([`RemoteError`]). Alle Implementierungen müssen die
//! Contract-Tests in `protonsync-testkit` bestehen.

use std::future::Future;
use std::io::{Read, Write};

use protonsync_domain::node::{NodeId, NodeName};
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;

use super::{EventCursor, RemoteBin, RemoteError, RemoteEvent};

/// Was zu einem neuen Foto beim Hochladen gehört.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPhoto {
    /// Dateiname.
    pub name: NodeName,
    /// Medientyp, z. B. `image/jpeg`.
    pub media_type: String,
    /// Größe in Bytes.
    pub size: u64,
    /// Aufnahmezeit, nach der die Zeitleiste sortiert.
    pub captured: Timestamp,
}

/// Abstrakte Fotobibliothek.
pub trait PhotoLibrary: Send + Sync {
    /// Alle Fotos der Zeitleiste (ohne Papierkorb), auch zugehörige Fotos.
    /// Ein Konto ohne Fotobereich liefert eine leere Liste.
    fn photos(&self) -> impl Future<Output = Result<Vec<Photo>, RemoteError>> + Send;

    /// Ein einzelnes Foto. [`RemoteError::NotFound`], wenn es nicht (mehr) da
    /// ist, im Papierkorb liegt oder kein Foto ist (z. B. ein Album).
    fn photo(&self, id: &NodeId) -> impl Future<Output = Result<Photo, RemoteError>> + Send;

    /// Ereignisse der Bibliothek nach `cursor` und der neue Cursor; mit `None`
    /// nur der aktuelle Cursor als Startpunkt.
    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send;

    /// Schreibt den Inhalt nach `target` und liefert die Anzahl Bytes.
    fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send;

    /// Lädt ein neues Foto hoch.
    fn upload<R: Read + Send>(
        &self,
        photo: &NewPhoto,
        content: R,
    ) -> impl Future<Output = Result<Photo, RemoteError>> + Send;

    /// Legt ein Foto in den Proton-Papierkorb.
    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Holt ein Foto aus dem Proton-Papierkorb zurück.
    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Löscht ein Foto aus dem Papierkorb endgültig; nur für Fotos im Papierkorb.
    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;
}

/// Eine geteilte Bibliothek (`Arc`) ist selbst eine Bibliothek.
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

/// Macht eine Fotobibliothek zum Papierkorb-Partner des [`RemoteBin`], damit
/// derselbe Papierkorb-Use-Case auch Fotos wiederherstellt und löscht.
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

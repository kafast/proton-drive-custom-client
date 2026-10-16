//! Port zum entfernten Speicher (Proton Drive oder ein Fake für Tests).
//!
//! Die Methoden sind asynchron. Statt `async fn` steht hier ausgeschrieben
//! `fn ... -> impl Future<Output = ...> + Send`: Das bedeutet dasselbe, verlangt
//! aber zusätzlich, dass das Ergebnis zwischen Threads wandern darf (`Send`).
//! Das braucht der Daemon, der mit mehreren Threads arbeitet.
//!
//! Alle Implementierungen müssen die Contract-Tests in `protonsync-testkit`
//! bestehen. Dort ist das erwartete Verhalten genau festgelegt.

use std::future::Future;
use std::io::{Read, Write};

use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::time::Timestamp;
use thiserror::Error;

/// Fehler beim Zugriff auf den entfernten Speicher.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteError {
    /// Knoten existiert nicht (oder liegt im Papierkorb).
    #[error("Knoten {0} nicht gefunden")]
    NotFound(NodeId),
    /// Im Zielordner gibt es schon einen Knoten mit diesem Namen.
    #[error("{0} existiert bereits im Zielordner")]
    AlreadyExists(NodeName),
    /// Ein Ordner wurde erwartet.
    #[error("{0} ist kein Ordner")]
    NotAFolder(NodeId),
    /// Eine Datei wurde erwartet.
    #[error("{0} ist keine Datei")]
    NotAFile(NodeId),
    /// Der angefragte Bereich beginnt hinter dem Dateiende.
    #[error("Bereich ab Byte {offset} liegt hinter dem Dateiende ({size} Bytes)")]
    InvalidRange {
        /// Angefragter Startpunkt.
        offset: u64,
        /// Tatsächliche Dateigröße.
        size: u64,
    },
    /// Verschieben ist so nicht möglich (z. B. Ordner in sich selbst).
    #[error("ungültiges Verschieben: {0}")]
    InvalidMove(String),
    /// Anmeldung fehlt oder ist abgelaufen.
    #[error("nicht angemeldet oder Sitzung abgelaufen")]
    Unauthorized,
    /// Lesen der Quelle bzw. Schreiben des Ziels ist fehlgeschlagen.
    #[error("Übertragung fehlgeschlagen: {0}")]
    Transfer(String),
    /// Vorübergehender Fehler (Netzwerk, Ratenbegrenzung); später erneut versuchen.
    #[error("vorübergehend nicht erreichbar: {0}")]
    Unavailable(String),
}

/// Position im Ereignisstrom des entfernten Speichers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCursor(String);

impl EventCursor {
    /// Erzeugt einen Cursor aus dem Wert des Speichers.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Der Cursor als Text (zum Speichern im Zustand).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Eine entfernte Änderung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEvent {
    /// Knoten wurde angelegt.
    Created(NodeId),
    /// Inhalt oder Metadaten (Name, Ort) haben sich geändert.
    Updated(NodeId),
    /// Knoten wurde in den Papierkorb gelegt.
    Trashed(NodeId),
}

/// Abstrakter entfernter Speicher.
pub trait RemoteStorage: Send + Sync {
    /// Der Wurzelordner des Speichers.
    fn root_folder(&self) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Liest die Metadaten eines Knotens.
    fn node(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Listet die Kinder eines Ordners (ohne Papierkorb), sortiert nach Name.
    fn list_children(
        &self,
        folder: &NodeId,
    ) -> impl Future<Output = Result<Vec<Node>, RemoteError>> + Send;

    /// Legt einen Ordner an.
    fn create_folder(
        &self,
        parent: &NodeId,
        name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Lädt eine neue Datei hoch. Gibt es den Namen schon: [`RemoteError::AlreadyExists`].
    ///
    /// Der Inhalt wird aus `content` gelesen (z. B. eine geöffnete Datei), damit
    /// auch große Dateien nicht komplett in den Speicher müssen. `size` ist die
    /// erwartete Größe in Bytes.
    fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Lädt eine neue Revision einer vorhandenen Datei hoch.
    fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Schreibt den gesamten Inhalt der aktuellen Revision nach `target` und
    /// liefert die Anzahl geschriebener Bytes.
    fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send;

    /// Lädt einen Teil des Inhalts (für den FUSE-Mount). Am Dateiende werden
    /// entsprechend weniger Bytes geliefert; ein Start hinter dem Ende ist ein Fehler.
    fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> impl Future<Output = Result<Vec<u8>, RemoteError>> + Send;

    /// Benennt einen Knoten um.
    fn rename(
        &self,
        id: &NodeId,
        new_name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Verschiebt einen Knoten in einen anderen Ordner.
    fn move_node(
        &self,
        id: &NodeId,
        new_parent: &NodeId,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Legt einen Knoten (samt Inhalt) in den entfernten Papierkorb.
    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Holt einen Knoten aus dem entfernten Papierkorb zurück.
    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Löscht einen Knoten aus dem entfernten Papierkorb endgültig.
    /// Nur für Knoten erlaubt, die im Papierkorb liegen.
    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Liefert die Ereignisse nach `cursor` und den neuen Cursor.
    ///
    /// Mit `None` gibt es keine Ereignisse, nur den aktuellen Cursor als Startpunkt.
    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send;
}

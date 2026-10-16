//! Port zum lokalen Dateisystem eines Sync-Paares.
//!
//! Eine Instanz steht für **einen** Wurzelordner; alle Pfade sind relativ dazu.
//! Die Methoden sind synchron: Lokale Dateizugriffe sind schnell genug, und der
//! Daemon kann sie bei Bedarf in einen eigenen Thread auslagern.

use std::io::{Read, Write};

use protonsync_domain::node::Fingerprint;
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;
use thiserror::Error;

/// Fehler beim Zugriff auf das lokale Dateisystem.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LocalError {
    /// Pfad existiert nicht.
    #[error("{0} nicht gefunden")]
    NotFound(RelativePath),
    /// Am Ziel liegt schon etwas.
    #[error("{0} existiert bereits")]
    AlreadyExists(RelativePath),
    /// Ein Ordner wurde erwartet oder ist im Weg.
    #[error("{0} ist kein Ordner")]
    NotAFolder(RelativePath),
    /// Sonstiger Ein-/Ausgabefehler (Rechte, voller Datenträger, …).
    #[error("Dateisystemfehler: {0}")]
    Io(String),
}

/// Art eines lokalen Eintrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalKind {
    /// Ordner.
    Folder,
    /// Datei mit Größe und Änderungszeit (ohne Prüfsumme, die ist teuer).
    File(Fingerprint),
}

/// Ein lokaler Eintrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    /// Pfad relativ zum Wurzelordner.
    pub path: RelativePath,
    /// Ordner oder Datei.
    pub kind: LocalKind,
}

/// Lokales Dateisystem eines Sync-Paares.
///
/// `type Reader` und `type Staging` sind „assoziierte Typen“: Jede
/// Implementierung legt fest, welcher konkrete Typ zurückkommt (z. B. eine
/// `std::fs::File`), ohne dass der Aufrufer ihn kennen muss.
pub trait LocalStorage: Send + Sync {
    /// Lesbarer Inhalt einer Datei.
    type Reader: Read + Send;
    /// Eine noch nicht sichtbare Datei, in die geschrieben wird.
    type Staging: Write + Send;

    /// Alle Einträge unterhalb des Wurzelordners (ohne die Wurzel selbst und ohne
    /// eigene Hilfsdateien), sortiert nach Pfad.
    ///
    /// # Errors
    /// [`LocalError::Io`] bei Lesefehlern.
    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError>;

    /// Ein einzelner Eintrag; `Ok(None)`, wenn es ihn nicht gibt.
    ///
    /// # Errors
    /// [`LocalError::Io`] bei Lesefehlern.
    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError>;

    /// Öffnet eine Datei zum Lesen.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] oder [`LocalError::Io`].
    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError>;

    /// SHA-1 des Inhalts als Hex in Kleinbuchstaben (wie bei Proton).
    ///
    /// # Errors
    /// [`LocalError::NotFound`] oder [`LocalError::Io`].
    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError>;

    /// Beginnt das Schreiben einer Datei. Erst [`LocalStorage::commit`] macht
    /// sie unter `path` sichtbar; so entsteht nie eine halb geschriebene Datei.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], wenn der Elternordner fehlt, sonst [`LocalError::Io`].
    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError>;

    /// Ersetzt `path` atomar durch die geschriebene Datei und setzt die
    /// Änderungszeit. Liefert die neuen Merkmale.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError>;

    /// Legt einen Ordner an (Elternordner müssen existieren). Existiert er schon, ist das kein Fehler.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::NotAFolder`] oder [`LocalError::Io`].
    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError>;

    /// Benennt um bzw. verschiebt innerhalb des Wurzelordners.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::AlreadyExists`] oder [`LocalError::Io`].
    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError>;

    /// Verschiebt Datei oder Ordner in den lokalen Papierkorb und liefert den
    /// Schlüssel, unter dem der Inhalt dort liegt.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] oder [`LocalError::Io`].
    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError>;

    /// Holt einen Eintrag aus dem Papierkorb nach `path` zurück.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::AlreadyExists`] oder [`LocalError::Io`].
    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError>;

    /// Löscht einen Eintrag endgültig aus dem Papierkorb.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] oder [`LocalError::Io`].
    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError>;
}

/// Ein geteilter Speicher (`Arc`) ist selbst ein Speicher; jede Methode leitet nur weiter.
impl<T: LocalStorage + ?Sized> LocalStorage for std::sync::Arc<T> {
    type Reader = T::Reader;
    type Staging = T::Staging;

    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError> {
        (**self).scan()
    }

    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError> {
        (**self).entry(path)
    }

    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError> {
        (**self).open(path)
    }

    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError> {
        (**self).content_hash(path)
    }

    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError> {
        (**self).stage(path)
    }

    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError> {
        (**self).commit(staging, path, modified)
    }

    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError> {
        (**self).create_folder(path)
    }

    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError> {
        (**self).rename(from, to)
    }

    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError> {
        (**self).move_to_trash(path)
    }

    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError> {
        (**self).restore_from_trash(stored_as, path)
    }

    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError> {
        (**self).purge_trash(stored_as)
    }
}

//! Sync-Paare: welcher lokale Ordner mit welchem entfernten Ordner wie abgeglichen wird.

use std::path::PathBuf;

use crate::node::NodeId;

/// Richtung der Synchronisation eines Sync-Paares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncDirection {
    /// Änderungen fließen in beide Richtungen (Standard).
    #[default]
    TwoWay,
    /// Nur entfernte Änderungen werden lokal übernommen.
    DownloadOnly,
    /// Nur lokale Änderungen werden hochgeladen.
    UploadOnly,
}

impl SyncDirection {
    /// Dürfen lokale Änderungen nach Proton übertragen werden?
    #[must_use]
    pub const fn allows_upload(self) -> bool {
        matches!(self, Self::TwoWay | Self::UploadOnly)
    }

    /// Dürfen entfernte Änderungen lokal übernommen werden?
    #[must_use]
    pub const fn allows_download(self) -> bool {
        matches!(self, Self::TwoWay | Self::DownloadOnly)
    }
}

/// Wie die Dateien lokal vorliegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// Alle Dateien liegen vollständig auf der Platte (Sync-Ordner).
    #[default]
    FullLocal,
    /// Dateien sind Platzhalter und werden bei Bedarf geladen (FUSE-Mount).
    OnDemand,
}

/// Kennung eines Sync-Paares in der lokalen Konfiguration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyncPairId(String);

impl SyncPairId {
    /// Erzeugt eine Kennung.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Die Kennung als Text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Verbindung eines lokalen Ordners mit einem Ordner in Proton Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPair {
    /// Kennung des Paares.
    pub id: SyncPairId,
    /// Lokaler Ordner bzw. Mountpunkt.
    pub local_root: PathBuf,
    /// Entfernter Ordner.
    pub remote_root: NodeId,
    /// Richtung des Abgleichs.
    pub direction: SyncDirection,
    /// Voll lokal oder bei Bedarf.
    pub storage_mode: StorageMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_way_is_the_default() {
        assert_eq!(SyncDirection::default(), SyncDirection::TwoWay);
    }

    #[test]
    fn direction_permissions() {
        assert!(SyncDirection::TwoWay.allows_upload());
        assert!(SyncDirection::TwoWay.allows_download());
        assert!(!SyncDirection::DownloadOnly.allows_upload());
        assert!(SyncDirection::DownloadOnly.allows_download());
        assert!(SyncDirection::UploadOnly.allows_upload());
        assert!(!SyncDirection::UploadOnly.allows_download());
    }
}

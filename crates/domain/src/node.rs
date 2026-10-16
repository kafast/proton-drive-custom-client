//! Knoten (Dateien und Ordner) und ihre Versionen.

use crate::DomainError;
use crate::time::Timestamp;

/// Maximale Länge eines Namens in Bytes (Grenze der üblichen Linux-Dateisysteme).
pub const MAX_NAME_BYTES: usize = 255;

/// Eindeutige, vom Speicher vergebene Kennung eines Knotens.
///
/// Ein „Newtype“: eine Struktur mit genau einem Feld. So kann der Compiler eine
/// `NodeId` nicht mit einem beliebigen anderen `String` verwechseln.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// Erzeugt eine Kennung aus dem Wert, den der Speicher geliefert hat.
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

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Ein geprüfter Datei- oder Ordnername (ein einzelnes Pfadsegment).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeName(String);

impl NodeName {
    /// Prüft und übernimmt einen Namen.
    ///
    /// # Errors
    /// [`DomainError::InvalidNodeName`], wenn der Name leer, `.` oder `..` ist,
    /// `/` oder ein Nullbyte enthält oder länger als [`MAX_NAME_BYTES`] ist.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let name = value.into();
        let reason = if name.is_empty() {
            Some("Name ist leer")
        } else if name == "." || name == ".." {
            Some("`.` und `..` sind reserviert")
        } else if name.contains('/') {
            Some("Name enthält `/`")
        } else if name.contains('\0') {
            Some("Name enthält ein Nullbyte")
        } else if name.len() > MAX_NAME_BYTES {
            Some("Name ist länger als 255 Bytes")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(DomainError::InvalidNodeName { name, reason }),
            None => Ok(Self(name)),
        }
    }

    /// Der Name als Text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Kennung einer Dateirevision, vom Speicher vergeben.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevisionId(String);

impl RevisionId {
    /// Erzeugt eine Revisionskennung.
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

/// Inhalt einer Datei zu einem Zeitpunkt: genug, um Änderungen zu erkennen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// Größe in Bytes.
    pub size: u64,
    /// Letzte Änderung laut Quelle.
    pub modified: Timestamp,
    /// Prüfsumme des Inhalts (z. B. SHA-256 als Hex), falls bekannt.
    pub content_hash: Option<String>,
}

/// Art eines Knotens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Ordner ohne eigenen Inhalt.
    Folder,
    /// Datei mit ihrer aktuellen Revision.
    File {
        /// Aktuelle Revision.
        revision: RevisionId,
        /// Merkmale des Inhalts dieser Revision.
        fingerprint: Fingerprint,
    },
}

/// Ein Knoten im entfernten oder lokalen Baum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Eindeutige Kennung.
    pub id: NodeId,
    /// Elternordner; `None` nur beim Wurzelordner.
    pub parent: Option<NodeId>,
    /// Name im Elternordner.
    pub name: NodeName,
    /// Ordner oder Datei.
    pub kind: NodeKind,
}

impl Node {
    /// `true`, wenn der Knoten ein Ordner ist.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, NodeKind::Folder)
    }

    /// Merkmale des Inhalts, nur bei Dateien vorhanden.
    #[must_use]
    pub fn fingerprint(&self) -> Option<&Fingerprint> {
        match &self.kind {
            NodeKind::File { fingerprint, .. } => Some(fingerprint),
            NodeKind::Folder => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        for name in ["Bericht.pdf", "Fotos 2026", "ümlaut", ".versteckt", "a..b"] {
            assert!(NodeName::new(name).is_ok(), "{name} sollte gültig sein");
        }
    }

    #[test]
    fn rejects_invalid_names() {
        let too_long = "x".repeat(MAX_NAME_BYTES + 1);
        for name in ["", ".", "..", "a/b", "nul\0byte", too_long.as_str()] {
            assert!(
                matches!(
                    NodeName::new(name),
                    Err(DomainError::InvalidNodeName { .. })
                ),
                "{name:?} sollte abgelehnt werden"
            );
        }
    }

    #[test]
    fn accepts_name_of_exactly_max_length() {
        assert!(NodeName::new("x".repeat(MAX_NAME_BYTES)).is_ok());
    }

    #[test]
    fn only_files_have_a_fingerprint() {
        let folder = Node {
            id: NodeId::new("1"),
            parent: None,
            name: NodeName::new("root").unwrap(),
            kind: NodeKind::Folder,
        };
        assert!(folder.is_folder());
        assert!(folder.fingerprint().is_none());
    }
}

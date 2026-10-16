//! Fotos aus dem Proton-Photos-Bereich (ADR 0012).
//!
//! Anders als Drive-Dateien bilden Fotos keinen Ordnerbaum, sondern eine flache
//! Zeitleiste. Lokal werden sie nach Aufnahmemonat abgelegt ([`layout`]).
//! Die Duplikaterkennung ([`duplicates`]) ist eine eigene, reine Funktion.

pub mod duplicates;
pub mod layout;

use crate::node::{NodeId, NodeName, RevisionId};
use crate::time::Timestamp;

/// Ein Foto (oder Video) in der Bibliothek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    /// Kennung in Proton.
    pub id: NodeId,
    /// Dateiname.
    pub name: NodeName,
    /// Aufnahmezeit laut Proton.
    pub captured: Timestamp,
    /// Wann das Foto hochgeladen wurde.
    pub uploaded: Timestamp,
    /// Aktuelle Revision; ändert sich nur bei neuem Inhalt.
    pub revision: RevisionId,
    /// Größe des Inhalts in Bytes.
    pub size: u64,
    /// SHA-1 des Inhalts als Hex in Kleinbuchstaben, falls bekannt.
    pub content_hash: Option<String>,
    /// Medientyp, z. B. `image/jpeg`.
    pub media_type: String,
    /// Bei zugehörigen Fotos (Video-Teil eines Live-Fotos, Serienbild) das Hauptfoto.
    pub main_photo: Option<NodeId>,
    /// Als Favorit markiert.
    pub favorite: bool,
    /// In wie vielen Alben das Foto liegt.
    pub albums: usize,
}

impl Photo {
    /// `true` für zugehörige Fotos, die mit ihrem Hauptfoto zusammen wandern.
    #[must_use]
    pub fn is_related(&self) -> bool {
        self.main_photo.is_some()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Ein Foto mit Standardwerten für Tests.
    pub(crate) fn photo(id: &str, name: &str, captured: i64) -> Photo {
        Photo {
            id: NodeId::new(id),
            name: NodeName::new(name).unwrap(),
            captured: Timestamp::from_unix_seconds(captured),
            uploaded: Timestamp::from_unix_seconds(captured + 60),
            revision: RevisionId::new(format!("{id}-r1")),
            size: 1000,
            content_hash: Some(format!("hash-{id}")),
            media_type: "image/jpeg".into(),
            main_photo: None,
            favorite: false,
            albums: 0,
        }
    }
}

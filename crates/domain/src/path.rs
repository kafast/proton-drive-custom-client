//! Pfade relativ zum Wurzelordner eines Sync-Paares.

use std::path::PathBuf;

use crate::DomainError;
use crate::node::NodeName;

/// Trennzeichen der gespeicherten Form. Unabhängig vom Betriebssystem, damit der
/// Zustand portabel bleibt; in echte Pfade wird über [`RelativePath::to_path_buf`] umgewandelt.
const SEPARATOR: char = '/';

/// Ein geprüfter, relativer Pfad aus gültigen [`NodeName`]-Segmenten.
///
/// Der leere Pfad steht für den Wurzelordner des Sync-Paares selbst.
/// Absolute Pfade und `..` sind ausgeschlossen, damit nichts außerhalb des
/// Sync-Ordners angesprochen werden kann.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RelativePath(String);

impl RelativePath {
    /// Der Wurzelordner.
    #[must_use]
    pub const fn root() -> Self {
        Self(String::new())
    }

    /// Prüft eine gespeicherte Form wie `Fotos/2026/bild.jpg`.
    ///
    /// # Errors
    /// [`DomainError::InvalidNodeName`], wenn ein Segment ungültig ist
    /// (also auch bei `..`, doppelten oder führenden `/`).
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        if value.is_empty() {
            return Ok(Self::root());
        }
        for segment in value.split(SEPARATOR) {
            NodeName::new(segment)?;
        }
        Ok(Self(value.to_owned()))
    }

    /// Hängt einen Namen an.
    #[must_use]
    pub fn join(&self, name: &NodeName) -> Self {
        if self.is_root() {
            Self(name.as_str().to_owned())
        } else {
            Self(format!("{}{SEPARATOR}{}", self.0, name.as_str()))
        }
    }

    /// `true` für den Wurzelordner.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Der übergeordnete Pfad; `None` beim Wurzelordner.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once(SEPARATOR) {
            Some((parent, _)) => Self(parent.to_owned()),
            None => Self::root(),
        })
    }

    /// Das letzte Segment; `None` beim Wurzelordner.
    #[must_use]
    pub fn file_name(&self) -> Option<NodeName> {
        if self.is_root() {
            return None;
        }
        let last = self.0.rsplit(SEPARATOR).next().unwrap_or(&self.0);
        // Alle Segmente wurden beim Erzeugen geprüft.
        NodeName::new(last).ok()
    }

    /// `true`, wenn `self` gleich `ancestor` ist oder darunter liegt.
    #[must_use]
    pub fn starts_with(&self, ancestor: &Self) -> bool {
        ancestor.is_root()
            || self.0 == ancestor.0
            || (self.0.starts_with(&ancestor.0)
                && self.0[ancestor.0.len()..].starts_with(SEPARATOR))
    }

    /// Anzahl der Segmente (Wurzel = 0), z. B. um Eltern vor Kindern zu bearbeiten.
    #[must_use]
    pub fn depth(&self) -> usize {
        if self.is_root() {
            0
        } else {
            self.0.split(SEPARATOR).count()
        }
    }

    /// Die gespeicherte Form mit `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Als Pfad des Betriebssystems (relativ).
    #[must_use]
    pub fn to_path_buf(&self) -> PathBuf {
        self.0.split(SEPARATOR).filter(|s| !s.is_empty()).collect()
    }
}

impl std::fmt::Display for RelativePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_root() {
            f.write_str(".")
        } else {
            f.write_str(&self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> RelativePath {
        RelativePath::parse(value).unwrap()
    }

    fn name(value: &str) -> NodeName {
        NodeName::new(value).unwrap()
    }

    #[test]
    fn parses_valid_paths() {
        assert!(path("").is_root());
        assert_eq!(path("a/b/c.txt").as_str(), "a/b/c.txt");
    }

    #[test]
    fn rejects_escaping_or_malformed_paths() {
        for value in ["/abs", "a//b", "a/../b", "..", "a/", "./a"] {
            assert!(
                RelativePath::parse(value).is_err(),
                "{value} sollte abgelehnt werden"
            );
        }
    }

    #[test]
    fn join_parent_and_file_name() {
        let file = RelativePath::root().join(&name("a")).join(&name("b.txt"));
        assert_eq!(file.as_str(), "a/b.txt");
        assert_eq!(file.file_name(), Some(name("b.txt")));
        assert_eq!(file.parent(), Some(path("a")));
        assert_eq!(path("a").parent(), Some(RelativePath::root()));
        assert_eq!(RelativePath::root().parent(), None);
        assert_eq!(RelativePath::root().file_name(), None);
    }

    #[test]
    fn starts_with_respects_segment_boundaries() {
        assert!(path("a/b").starts_with(&path("a")));
        assert!(path("a").starts_with(&path("a")));
        assert!(path("a").starts_with(&RelativePath::root()));
        assert!(!path("ab/c").starts_with(&path("a")));
    }

    #[test]
    fn depth_counts_segments() {
        assert_eq!(RelativePath::root().depth(), 0);
        assert_eq!(path("a/b/c").depth(), 3);
    }

    #[test]
    fn converts_to_os_path() {
        assert_eq!(path("a/b").to_path_buf(), PathBuf::from("a").join("b"));
        assert_eq!(RelativePath::root().to_path_buf(), PathBuf::new());
        assert_eq!(RelativePath::root().to_string(), ".");
    }
}

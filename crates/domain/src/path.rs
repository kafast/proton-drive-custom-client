//! Paths relative to the root folder of a sync pair.

use std::path::PathBuf;

use crate::DomainError;
use crate::node::NodeName;

/// Separator of the stored form. Independent of the operating system so that the
/// state stays portable; conversion to real paths goes through [`RelativePath::to_path_buf`].
const SEPARATOR: char = '/';

/// A validated relative path made of valid [`NodeName`] segments.
///
/// The empty path stands for the root folder of the sync pair itself.
/// Absolute paths and `..` are excluded so that nothing outside the
/// sync folder can be addressed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RelativePath(String);

impl RelativePath {
    /// The root folder.
    #[must_use]
    pub const fn root() -> Self {
        Self(String::new())
    }

    /// Validates a stored form such as `Photos/2026/image.jpg`.
    ///
    /// # Errors
    /// [`DomainError::InvalidNodeName`] if a segment is invalid
    /// (so also for `..` and doubled or leading `/`).
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        if value.is_empty() {
            return Ok(Self::root());
        }
        for segment in value.split(SEPARATOR) {
            NodeName::new(segment)?;
        }
        Ok(Self(value.to_owned()))
    }

    /// Appends a name.
    #[must_use]
    pub fn join(&self, name: &NodeName) -> Self {
        if self.is_root() {
            Self(name.as_str().to_owned())
        } else {
            Self(format!("{}{SEPARATOR}{}", self.0, name.as_str()))
        }
    }

    /// `true` for the root folder.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The parent path; `None` for the root folder.
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

    /// The last segment; `None` for the root folder.
    #[must_use]
    pub fn file_name(&self) -> Option<NodeName> {
        if self.is_root() {
            return None;
        }
        let last = self.0.rsplit(SEPARATOR).next().unwrap_or(&self.0);
        // All segments were validated on creation.
        NodeName::new(last).ok()
    }

    /// `true` if `self` equals `ancestor` or lies below it.
    #[must_use]
    pub fn starts_with(&self, ancestor: &Self) -> bool {
        ancestor.is_root()
            || self.0 == ancestor.0
            || (self.0.starts_with(&ancestor.0)
                && self.0[ancestor.0.len()..].starts_with(SEPARATOR))
    }

    /// Number of segments (root = 0), e.g. to process parents before children.
    #[must_use]
    pub fn depth(&self) -> usize {
        if self.is_root() {
            0
        } else {
            self.0.split(SEPARATOR).count()
        }
    }

    /// The stored form with `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// As an operating system path (relative).
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
                "{value} should be rejected"
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

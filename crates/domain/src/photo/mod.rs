//! Photos from the Proton Photos area (ADR 0012).
//!
//! Unlike Drive files, photos do not form a folder tree but a flat
//! timeline. Locally they are filed by month of capture ([`layout`]).
//! Duplicate detection ([`duplicates`]) is a separate, pure function.

pub mod duplicates;
pub mod layout;

use crate::node::{NodeId, NodeName, RevisionId};
use crate::time::Timestamp;

/// A photo (or video) in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    /// Id in Proton.
    pub id: NodeId,
    /// File name.
    pub name: NodeName,
    /// Capture time according to Proton.
    pub captured: Timestamp,
    /// When the photo was uploaded.
    pub uploaded: Timestamp,
    /// Current revision; only changes with new content.
    pub revision: RevisionId,
    /// Size of the content in bytes.
    pub size: u64,
    /// SHA-1 of the content as lowercase hex, if known.
    pub content_hash: Option<String>,
    /// Media type, e.g. `image/jpeg`.
    pub media_type: String,
    /// For related photos (video part of a live photo, burst shot) the main photo.
    pub main_photo: Option<NodeId>,
    /// Marked as favourite.
    pub favorite: bool,
    /// How many albums the photo is in.
    pub albums: usize,
}

impl Photo {
    /// `true` for related photos that move together with their main photo.
    #[must_use]
    pub fn is_related(&self) -> bool {
        self.main_photo.is_some()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A photo with default values for tests.
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

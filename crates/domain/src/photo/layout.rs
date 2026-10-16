//! Where photos are stored locally and which files count as photos.
//!
//! Layout: `YYYY/MM/name` by capture time. If the name already exists in the month,
//! a piece of the id is appended ([`distinct_name`]).

use crate::job::LocalDateTime;
use crate::node::{NodeId, NodeName};
use crate::path::RelativePath;

/// This many characters of the id go into a fallback name.
const ID_SUFFIX_CHARS: usize = 8;

/// Known extensions of images and videos with their media type.
const MEDIA_TYPES: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("avif", "image/avif"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("bmp", "image/bmp"),
    ("dng", "image/x-adobe-dng"),
    ("cr2", "image/x-canon-cr2"),
    ("cr3", "image/x-canon-cr3"),
    ("nef", "image/x-nikon-nef"),
    ("arw", "image/x-sony-arw"),
    ("orf", "image/x-olympus-orf"),
    ("rw2", "image/x-panasonic-rw2"),
    ("raf", "image/x-fuji-raf"),
    ("mp4", "video/mp4"),
    ("m4v", "video/x-m4v"),
    ("mov", "video/quicktime"),
    ("3gp", "video/3gpp"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("avi", "video/x-msvideo"),
];

/// Month folder for a capture time, e.g. `2026/10`.
#[must_use]
pub fn month_folder(captured: LocalDateTime) -> RelativePath {
    let path = format!("{:04}/{:02}", captured.year(), captured.month());
    // Year and month always give valid segments; the fallback is only a formality.
    RelativePath::parse(&path).unwrap_or_default()
}

/// Media type of a file by its extension; `None` if it is not a photo or video.
#[must_use]
pub fn media_type(name: &NodeName) -> Option<&'static str> {
    let (stem, extension) = name.as_str().rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let extension = extension.to_ascii_lowercase();
    MEDIA_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, media_type)| *media_type)
}

/// Fallback name with a piece of the id, e.g. `IMG_0001 (a1b2c3d4).jpg`.
///
/// Characters not allowed in names (`/`) are removed from the id;
/// if the name gets too long, the stem is shortened.
#[must_use]
pub fn distinct_name(name: &NodeName, id: &NodeId) -> NodeName {
    let suffix: String = id
        .as_str()
        .chars()
        .rev()
        .filter(char::is_ascii_alphanumeric)
        .take(ID_SUFFIX_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let (stem, extension) = match name.as_str().rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name.as_str(), None),
    };
    let marker = format!(" ({suffix})");
    let extension_len = extension.map_or(0, |e| e.len() + 1);
    let room = crate::node::MAX_NAME_BYTES.saturating_sub(marker.len() + extension_len);
    let stem = truncate_to_char_boundary(stem, room);
    let candidate = match extension {
        Some(extension) => format!("{stem}{marker}.{extension}"),
        None => format!("{stem}{marker}"),
    };
    NodeName::new(candidate).unwrap_or_else(|_| name.clone())
}

fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> NodeName {
        NodeName::new(value).unwrap()
    }

    #[test]
    fn photos_are_filed_by_month() {
        let captured = LocalDateTime::new(2026, 3, 9, 14, 30).unwrap();
        assert_eq!(month_folder(captured).as_str(), "2026/03");
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(media_type(&name("IMG_1.JPG")), Some("image/jpeg"));
        assert_eq!(media_type(&name("clip.mov")), Some("video/quicktime"));
        assert_eq!(media_type(&name("raw.DNG")), Some("image/x-adobe-dng"));
        assert_eq!(media_type(&name("notes.txt")), None);
        assert_eq!(media_type(&name(".jpg")), None);
        assert_eq!(media_type(&name("no_extension")), None);
    }

    #[test]
    fn distinct_names_keep_the_extension() {
        let id = NodeId::new("vol~link/abcdef123456==");
        assert_eq!(
            distinct_name(&name("IMG_0001.jpg"), &id).as_str(),
            "IMG_0001 (ef123456).jpg"
        );
        assert_eq!(
            distinct_name(&name("README"), &id).as_str(),
            "README (ef123456)"
        );
    }

    #[test]
    fn distinct_names_stay_within_the_length_limit() {
        let long = format!("{}.jpg", "ä".repeat(125));
        let result = distinct_name(&name(&long), &NodeId::new("12345678"));
        assert!(result.as_str().len() <= crate::node::MAX_NAME_BYTES);
        assert!(result.as_str().ends_with(" (12345678).jpg"), "{result}");
    }
}

//! Duplicate detection: finds groups of identical photos and suggests which one
//! stays. It never deletes anything itself (ADR 0012).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use super::Photo;
use crate::node::NodeId;
use crate::time::Timestamp;

/// Why photos count as duplicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DuplicateReason {
    /// Same content (SHA-1 and size): certainly the same photo.
    SameContent,
    /// Same name and same capture second, but different content
    /// (e.g. recompressed): probably the same photo.
    SameNameAndCaptureTime,
}

/// A group of duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Why the photos belong together.
    pub reason: DuplicateReason,
    /// Suggestion: keep this photo.
    pub keep: NodeId,
    /// The remaining photos of the group (candidates for deletion).
    pub others: Vec<NodeId>,
    /// Capture time of the kept photo (for sorting and display).
    pub captured: Timestamp,
}

impl DuplicateGroup {
    /// All photos of the group, the kept one first.
    pub fn members(&self) -> impl Iterator<Item = &NodeId> {
        std::iter::once(&self.keep).chain(&self.others)
    }
}

/// Searches for duplicates among `photos`, newest captures first.
///
/// Each photo is in at most one group; same content takes precedence over
/// same name. Related photos (e.g. the video part of a live photo)
/// are not considered because they move with their main photo.
#[must_use]
pub fn find_duplicates(photos: &[Photo]) -> Vec<DuplicateGroup> {
    let candidates: Vec<&Photo> = photos.iter().filter(|p| !p.is_related()).collect();
    let mut groups = Vec::new();
    let mut grouped = BTreeSet::new();

    let mut by_content: BTreeMap<(&str, u64), Vec<&Photo>> = BTreeMap::new();
    for photo in &candidates {
        if let Some(hash) = &photo.content_hash {
            by_content
                .entry((hash.as_str(), photo.size))
                .or_default()
                .push(photo);
        }
    }
    collect(
        by_content.into_values(),
        DuplicateReason::SameContent,
        &mut grouped,
        &mut groups,
    );

    let mut by_name: BTreeMap<(String, Timestamp), Vec<&Photo>> = BTreeMap::new();
    for photo in candidates.iter().filter(|p| !grouped.contains(&p.id)) {
        by_name
            .entry((photo.name.as_str().to_lowercase(), photo.captured))
            .or_default()
            .push(photo);
    }
    collect(
        by_name.into_values(),
        DuplicateReason::SameNameAndCaptureTime,
        &mut grouped,
        &mut groups,
    );

    groups.sort_by(|a, b| {
        b.captured
            .cmp(&a.captured)
            .then_with(|| a.keep.cmp(&b.keep))
    });
    groups
}

fn collect<'a>(
    buckets: impl Iterator<Item = Vec<&'a Photo>>,
    reason: DuplicateReason,
    grouped: &mut BTreeSet<NodeId>,
    groups: &mut Vec<DuplicateGroup>,
) {
    for mut members in buckets.filter(|members| members.len() > 1) {
        members.sort_by_key(|photo| keep_rank(photo));
        let keep = members[0];
        grouped.extend(members.iter().map(|photo| photo.id.clone()));
        groups.push(DuplicateGroup {
            reason,
            keep: keep.id.clone(),
            others: members[1..].iter().map(|photo| photo.id.clone()).collect(),
            captured: keep.captured,
        });
    }
}

/// Order for the suggestion: favourite, then album member, then the
/// photo uploaded first; on a tie the id decides.
fn keep_rank(photo: &Photo) -> (Reverse<bool>, Reverse<bool>, Timestamp, NodeId) {
    (
        Reverse(photo.favorite),
        Reverse(photo.albums > 0),
        photo.uploaded,
        photo.id.clone(),
    )
}

/// Why a deletion request for duplicates is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DuplicateSelectionError {
    /// The id is not in any group.
    #[error("{0} is not a known duplicate")]
    NotADuplicate(NodeId),
    /// All photos of this group would be gone; one must stay.
    #[error("all photos of the group of {0} would be deleted; at least one must stay")]
    WholeGroup(NodeId),
}

/// Checks whether `selected` may be deleted: every id must be in a group,
/// and at least one photo of every group must remain.
///
/// # Errors
/// [`DuplicateSelectionError`] on the first violation.
pub fn check_selection(
    groups: &[DuplicateGroup],
    selected: &[NodeId],
) -> Result<(), DuplicateSelectionError> {
    let selected: BTreeSet<&NodeId> = selected.iter().collect();
    for id in &selected {
        if !groups.iter().any(|group| group.members().any(|m| m == *id)) {
            return Err(DuplicateSelectionError::NotADuplicate((*id).clone()));
        }
    }
    for group in groups {
        if group.members().all(|member| selected.contains(member)) {
            return Err(DuplicateSelectionError::WholeGroup(group.keep.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::photo::test_support::photo;

    fn ids(group: &DuplicateGroup) -> Vec<&str> {
        group.members().map(NodeId::as_str).collect()
    }

    #[test]
    fn same_content_forms_a_group_and_keeps_the_first_upload() {
        let mut a = photo("a", "IMG_1.jpg", 100);
        let mut b = photo("b", "copy.jpg", 500);
        let c = photo("c", "IMG_2.jpg", 100);
        a.content_hash = Some("x".into());
        b.content_hash = Some("x".into());
        b.uploaded = Timestamp::from_unix_seconds(10);
        let groups = find_duplicates(&[a, b, c]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].reason, DuplicateReason::SameContent);
        assert_eq!(ids(&groups[0]), ["b", "a"]);
    }

    #[test]
    fn favourites_and_album_members_are_kept() {
        let mut a = photo("a", "x.jpg", 1);
        let mut b = photo("b", "x.jpg", 1);
        let mut c = photo("c", "x.jpg", 1);
        for p in [&mut a, &mut b, &mut c] {
            p.content_hash = Some("same".into());
        }
        b.albums = 2;
        assert_eq!(
            ids(&find_duplicates(&[a.clone(), b.clone(), c.clone()])[0]),
            ["b", "a", "c"]
        );
        c.favorite = true;
        assert_eq!(ids(&find_duplicates(&[a, b, c])[0]), ["c", "b", "a"]);
    }

    #[test]
    fn same_name_and_second_counts_only_without_content_match() {
        let mut a = photo("a", "IMG_7.JPG", 42);
        let b = photo("b", "img_7.jpg", 42);
        let c = photo("c", "img_7.jpg", 43);
        a.size = 5;
        let groups = find_duplicates(&[a, b, c]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].reason, DuplicateReason::SameNameAndCaptureTime);
        assert_eq!(ids(&groups[0]), ["a", "b"]);
    }

    #[test]
    fn related_photos_and_missing_hashes_are_ignored() {
        let mut main = photo("main", "live.heic", 7);
        let mut video = photo("video", "live.mov", 7);
        let mut other_video = photo("v2", "live2.mov", 8);
        video.main_photo = Some(main.id.clone());
        other_video.main_photo = Some(main.id.clone());
        video.content_hash = Some("v".into());
        other_video.content_hash = Some("v".into());
        main.content_hash = None;
        let mut copy = photo("copy", "copy.heic", 9);
        copy.content_hash = None;
        assert_eq!(
            find_duplicates(&[main, video, other_video, copy]),
            Vec::<DuplicateGroup>::new()
        );
    }

    #[test]
    fn newest_groups_come_first() {
        let mut photos = Vec::new();
        for (id, captured, hash) in [
            ("a", 1, "h1"),
            ("b", 1, "h1"),
            ("c", 9, "h2"),
            ("d", 9, "h2"),
        ] {
            let mut p = photo(id, &format!("{id}.jpg"), captured);
            p.content_hash = Some(hash.into());
            photos.push(p);
        }
        let groups = find_duplicates(&photos);
        assert_eq!(groups[0].keep.as_str(), "c");
        assert_eq!(groups[1].keep.as_str(), "a");
    }

    #[test]
    fn selections_must_leave_one_photo_per_group() {
        let mut a = photo("a", "x.jpg", 1);
        let mut b = photo("b", "y.jpg", 1);
        a.content_hash = Some("h".into());
        b.content_hash = Some("h".into());
        let groups = find_duplicates(&[a, b, photo("c", "z.jpg", 2)]);
        let id = NodeId::new;
        assert_eq!(check_selection(&groups, &[id("b")]), Ok(()));
        assert_eq!(check_selection(&groups, &[id("a")]), Ok(()));
        assert_eq!(
            check_selection(&groups, &[id("a"), id("b")]),
            Err(DuplicateSelectionError::WholeGroup(id("a")))
        );
        assert_eq!(
            check_selection(&groups, &[id("c")]),
            Err(DuplicateSelectionError::NotADuplicate(id("c")))
        );
    }

    fn arbitrary_photos() -> impl Strategy<Value = Vec<Photo>> {
        prop::collection::vec(
            (0u8..4, 0u8..3, 0i64..3, 0u64..2, any::<bool>(), 0u8..6),
            0..12,
        )
        .prop_map(|specs| {
            specs
                .into_iter()
                .enumerate()
                .map(|(index, (hash, name, captured, size, favorite, related))| {
                    let mut p = photo(&format!("p{index}"), &format!("n{name}.jpg"), captured);
                    p.content_hash = (hash > 0).then(|| format!("h{hash}"));
                    p.size = size;
                    p.favorite = favorite;
                    p.main_photo = (related == 0).then(|| NodeId::new("main"));
                    p
                })
                .collect()
        })
    }

    proptest! {
        #[test]
        fn groups_are_disjoint_and_consistent(photos in arbitrary_photos()) {
            let groups = find_duplicates(&photos);
            let mut seen = BTreeSet::new();
            for group in &groups {
                prop_assert!(!group.others.is_empty());
                let members: Vec<&Photo> = group
                    .members()
                    .map(|id| photos.iter().find(|p| &p.id == id).unwrap())
                    .collect();
                for member in &members {
                    prop_assert!(seen.insert(member.id.clone()), "duplicate: {}", member.id);
                    prop_assert!(!member.is_related());
                }
                let first = members[0];
                for member in &members[1..] {
                    match group.reason {
                        DuplicateReason::SameContent => {
                            prop_assert_eq!(&member.content_hash, &first.content_hash);
                            prop_assert!(member.content_hash.is_some());
                            prop_assert_eq!(member.size, first.size);
                        }
                        DuplicateReason::SameNameAndCaptureTime => {
                            prop_assert_eq!(member.name.as_str().to_lowercase(), first.name.as_str().to_lowercase());
                            prop_assert_eq!(member.captured, first.captured);
                        }
                    }
                    prop_assert!(keep_rank(first) <= keep_rank(member));
                }
                // The group may never be deleted entirely.
                let all: Vec<NodeId> = group.members().cloned().collect();
                prop_assert!(check_selection(&groups, &all).is_err());
                prop_assert!(check_selection(&groups, &group.others).is_ok());
            }
        }
    }
}

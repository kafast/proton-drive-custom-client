//! "Always keep local" and automatic freeing of space in the cache
//! of the virtual drive.

use std::collections::BTreeMap;

use crate::DomainError;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Whether a file or folder is kept local permanently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinState {
    /// May be evicted once the period has passed (default).
    #[default]
    Unpinned,
    /// Always stays local ("Always keep local").
    Pinned,
}

/// "Always keep local" and "Only on demand" rules for the paths of a drive.
///
/// A rule applies to the path itself and everything below it. The closest
/// rule wins (file before parent folder before grandparent folder ...); without a rule
/// the drive's default applies.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinRules {
    rules: BTreeMap<RelativePath, PinState>,
}

impl PinRules {
    /// No rules.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a rule for `path` (overwrites an existing one).
    pub fn set(&mut self, path: RelativePath, state: PinState) {
        self.rules.insert(path, state);
    }

    /// Removes the rule for `path`; the parent folder applies again.
    pub fn clear(&mut self, path: &RelativePath) {
        self.rules.remove(path);
    }

    /// Removes all rules for `path` and below (e.g. after deleting
    /// a folder or before a new rule for the whole folder).
    pub fn clear_below(&mut self, path: &RelativePath) {
        self.rules.retain(|rule, _| !rule.starts_with(path));
    }

    /// The explicitly set rules, sorted by path.
    pub fn iter(&self) -> impl Iterator<Item = (&RelativePath, PinState)> {
        self.rules.iter().map(|(path, state)| (path, *state))
    }

    /// What applies to `path`.
    #[must_use]
    pub fn effective(&self, path: &RelativePath, default: PinState) -> PinState {
        let mut current = Some(path.clone());
        while let Some(candidate) = current {
            if let Some(state) = self.rules.get(&candidate) {
                return *state;
            }
            current = candidate.parent();
        }
        default
    }
}

/// Rule for automatically evicting locally downloaded files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionPolicy {
    unused_days: u32,
    max_bytes: u64,
}

/// What is known about a locally cached file in order to decide on
/// eviction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFile<K> {
    /// Id in the cache (chosen by the caller).
    pub key: K,
    /// Pinned or not (according to [`PinRules::effective`]).
    pub pin: PinState,
    /// Last access.
    pub last_access: Timestamp,
    /// Space used in bytes.
    pub bytes: u64,
    /// `true` if local changes are not yet in Proton.
    pub changed_locally: bool,
    /// `true` while the file is open.
    pub open: bool,
}

impl<K> CachedFile<K> {
    /// May it be evicted at all? Never if pinned, changed or
    /// open; otherwise a copy that is still missing elsewhere would be lost, or
    /// a program would lose its data.
    #[must_use]
    pub fn is_evictable(&self) -> bool {
        self.pin == PinState::Unpinned && !self.changed_locally && !self.open
    }
}

impl EvictionPolicy {
    /// Files are evicted when they have not been used for `unused_days` days
    /// or the cache uses more than `max_bytes`.
    ///
    /// # Errors
    /// [`DomainError::ZeroDays`] for `0` days.
    pub const fn new(unused_days: u32, max_bytes: u64) -> Result<Self, DomainError> {
        if unused_days == 0 {
            return Err(DomainError::ZeroDays);
        }
        Ok(Self {
            unused_days,
            max_bytes,
        })
    }

    /// Period in days.
    #[must_use]
    pub const fn unused_days(self) -> u32 {
        self.unused_days
    }

    /// Size limit in bytes.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// Which files should be evicted now, oldest first.
    ///
    /// First all evictable files not used within the period; then, as long as
    /// the cache is above the limit, further evictable files in order of
    /// last access. Pinned, changed and open files always stay,
    /// even if the limit then remains exceeded.
    #[must_use]
    pub fn plan<K: Clone>(self, files: &[CachedFile<K>], now: Timestamp) -> Vec<K> {
        let mut candidates: Vec<&CachedFile<K>> =
            files.iter().filter(|file| file.is_evictable()).collect();
        candidates.sort_by_key(|file| file.last_access);
        let mut remaining: u64 = files.iter().map(|file| file.bytes).sum();
        let mut chosen = Vec::new();
        for file in candidates {
            let expired = now >= file.last_access.plus_days(self.unused_days);
            if !expired && remaining <= self.max_bytes {
                // Sorted by access: all remaining ones are newer, and the limit fits.
                break;
            }
            remaining = remaining.saturating_sub(file.bytes);
            chosen.push(file.key.clone());
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SECONDS_PER_DAY;

    const NOW: Timestamp = Timestamp::from_unix_seconds(100 * SECONDS_PER_DAY);

    fn days_ago(days: i64) -> Timestamp {
        Timestamp::from_unix_seconds(NOW.unix_seconds() - days * SECONDS_PER_DAY)
    }

    fn path(text: &str) -> RelativePath {
        RelativePath::parse(text).unwrap()
    }

    fn file(key: u32, last_used_days_ago: i64, bytes: u64) -> CachedFile<u32> {
        CachedFile {
            key,
            pin: PinState::Unpinned,
            last_access: days_ago(last_used_days_ago),
            bytes,
            changed_locally: false,
            open: false,
        }
    }

    fn policy(max_bytes: u64) -> EvictionPolicy {
        EvictionPolicy::new(30, max_bytes).unwrap()
    }

    #[test]
    fn closest_rule_wins_and_default_applies_without_rule() {
        let mut rules = PinRules::new();
        assert_eq!(
            rules.effective(&path("a/b.txt"), PinState::Unpinned),
            PinState::Unpinned
        );
        rules.set(path("a"), PinState::Pinned);
        rules.set(path("a/tmp"), PinState::Unpinned);
        assert_eq!(
            rules.effective(&path("a/b.txt"), PinState::Unpinned),
            PinState::Pinned
        );
        assert_eq!(
            rules.effective(&path("a/tmp/x"), PinState::Unpinned),
            PinState::Unpinned
        );
        assert_eq!(
            rules.effective(&path("other"), PinState::Pinned),
            PinState::Pinned
        );
        rules.set(RelativePath::root(), PinState::Pinned);
        assert_eq!(
            rules.effective(&path("other"), PinState::Unpinned),
            PinState::Pinned
        );
    }

    #[test]
    fn clearing_rules_falls_back_to_parents() {
        let mut rules = PinRules::new();
        rules.set(path("a"), PinState::Pinned);
        rules.set(path("a/b"), PinState::Unpinned);
        rules.set(path("a/b/c"), PinState::Pinned);
        rules.set(path("ab"), PinState::Pinned);
        rules.clear(&path("a/b/c"));
        assert_eq!(
            rules.effective(&path("a/b/c"), PinState::Pinned),
            PinState::Unpinned
        );
        rules.clear_below(&path("a"));
        assert_eq!(rules.iter().count(), 1, "only `ab` remains");
    }

    #[test]
    fn evicts_files_unused_for_the_period() {
        let files = [file(1, 31, 10), file(2, 29, 10), file(3, 30, 10)];
        assert_eq!(policy(u64::MAX).plan(&files, NOW), vec![1, 3]);
    }

    #[test]
    fn never_evicts_pinned_changed_or_open_files() {
        let pinned = CachedFile {
            pin: PinState::Pinned,
            ..file(1, 90, 10)
        };
        let changed = CachedFile {
            changed_locally: true,
            ..file(2, 90, 10)
        };
        let open = CachedFile {
            open: true,
            ..file(3, 90, 10)
        };
        assert_eq!(
            policy(0).plan(&[pinned, changed, open], NOW),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn over_the_limit_the_least_recently_used_go_first() {
        let files = [
            file(1, 1, 50),
            file(2, 3, 50),
            file(3, 2, 50),
            file(4, 0, 50),
        ];
        // 200 bytes used, 120 allowed: the two oldest (2, then 3) are enough.
        assert_eq!(policy(120).plan(&files, NOW), vec![2, 3]);
    }

    #[test]
    fn limit_counts_files_that_must_stay() {
        let pinned = CachedFile {
            pin: PinState::Pinned,
            ..file(1, 0, 100)
        };
        let files = [pinned, file(2, 5, 10), file(3, 1, 10)];
        // The pinned file alone is already over the limit: all evictable ones go.
        assert_eq!(policy(50).plan(&files, NOW), vec![2, 3]);
    }

    #[test]
    fn zero_days_are_rejected() {
        assert_eq!(EvictionPolicy::new(0, 1), Err(DomainError::ZeroDays));
        assert_eq!(policy(7).unused_days(), 30);
        assert_eq!(policy(7).max_bytes(), 7);
    }
}

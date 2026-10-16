//! „Immer lokal halten“ und automatische Speicherfreigabe im Zwischenspeicher
//! des virtuellen Laufwerks.

use std::collections::BTreeMap;

use crate::DomainError;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Ob eine Datei oder ein Ordner dauerhaft lokal gehalten wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinState {
    /// Darf nach Ablauf der Frist freigegeben werden (Standard).
    #[default]
    Unpinned,
    /// Bleibt immer lokal („Immer lokal halten“).
    Pinned,
}

/// Regeln „Immer lokal halten“ bzw. „Nur bei Bedarf“ für Pfade eines Laufwerks.
///
/// Eine Regel gilt für den Pfad selbst und alles darunter. Die nächstgelegene
/// Regel gewinnt (Datei vor Elternordner vor Großelternordner …); ohne Regel
/// gilt die Voreinstellung des Laufwerks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinRules {
    rules: BTreeMap<RelativePath, PinState>,
}

impl PinRules {
    /// Keine Regeln.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Setzt eine Regel für `path` (überschreibt eine vorhandene).
    pub fn set(&mut self, path: RelativePath, state: PinState) {
        self.rules.insert(path, state);
    }

    /// Entfernt die Regel für `path`; dann gilt wieder der Elternordner.
    pub fn clear(&mut self, path: &RelativePath) {
        self.rules.remove(path);
    }

    /// Entfernt alle Regeln für `path` und darunter (z. B. nach dem Löschen
    /// eines Ordners oder vor einer neuen Regel für den ganzen Ordner).
    pub fn clear_below(&mut self, path: &RelativePath) {
        self.rules.retain(|rule, _| !rule.starts_with(path));
    }

    /// Die ausdrücklich gesetzten Regeln, sortiert nach Pfad.
    pub fn iter(&self) -> impl Iterator<Item = (&RelativePath, PinState)> {
        self.rules.iter().map(|(path, state)| (path, *state))
    }

    /// Was für `path` gilt.
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

/// Regel für die automatische Freigabe lokal geladener Dateien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionPolicy {
    unused_days: u32,
    max_bytes: u64,
}

/// Was über eine lokal zwischengespeicherte Datei bekannt ist, um über die
/// Freigabe zu entscheiden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFile<K> {
    /// Kennung im Zwischenspeicher (vom Aufrufer gewählt).
    pub key: K,
    /// Angeheftet oder nicht (nach [`PinRules::effective`]).
    pub pin: PinState,
    /// Letzter Zugriff.
    pub last_access: Timestamp,
    /// Belegter Platz in Bytes.
    pub bytes: u64,
    /// `true`, wenn lokale Änderungen noch nicht in Proton sind.
    pub changed_locally: bool,
    /// `true`, solange die Datei geöffnet ist.
    pub open: bool,
}

impl<K> CachedFile<K> {
    /// Darf überhaupt freigegeben werden? Nie, wenn angeheftet, geändert oder
    /// geöffnet; sonst ginge eine Kopie verloren, die woanders noch fehlt, oder
    /// ein Programm verlöre seine Daten.
    #[must_use]
    pub fn is_evictable(&self) -> bool {
        self.pin == PinState::Unpinned && !self.changed_locally && !self.open
    }
}

impl EvictionPolicy {
    /// Dateien werden freigegeben, wenn sie `unused_days` Tage nicht benutzt
    /// wurden oder der Zwischenspeicher mehr als `max_bytes` belegt.
    ///
    /// # Errors
    /// [`DomainError::ZeroDays`] bei `0` Tagen.
    pub const fn new(unused_days: u32, max_bytes: u64) -> Result<Self, DomainError> {
        if unused_days == 0 {
            return Err(DomainError::ZeroDays);
        }
        Ok(Self {
            unused_days,
            max_bytes,
        })
    }

    /// Frist in Tagen.
    #[must_use]
    pub const fn unused_days(self) -> u32 {
        self.unused_days
    }

    /// Größenlimit in Bytes.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// Welche Dateien jetzt freigegeben werden sollen, älteste zuerst.
    ///
    /// Zuerst alle freigebbaren, die seit der Frist nicht benutzt wurden; dann,
    /// solange der Zwischenspeicher über dem Limit liegt, weitere freigebbare
    /// in der Reihenfolge des letzten Zugriffs. Angeheftete, geänderte und
    /// geöffnete Dateien bleiben immer, auch wenn das Limit dann überschritten bleibt.
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
                // Sortiert nach Zugriff: Alle weiteren sind jünger, und das Limit passt.
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
        assert_eq!(rules.iter().count(), 1, "nur `ab` bleibt");
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
        assert_eq!(policy(0).plan(&[pinned, changed, open], NOW), Vec::<u32>::new());
    }

    #[test]
    fn over_the_limit_the_least_recently_used_go_first() {
        let files = [
            file(1, 1, 50),
            file(2, 3, 50),
            file(3, 2, 50),
            file(4, 0, 50),
        ];
        // 200 Bytes belegt, 120 erlaubt: die beiden ältesten (2, dann 3) reichen.
        assert_eq!(policy(120).plan(&files, NOW), vec![2, 3]);
    }

    #[test]
    fn limit_counts_files_that_must_stay() {
        let pinned = CachedFile {
            pin: PinState::Pinned,
            ..file(1, 0, 100)
        };
        let files = [pinned, file(2, 5, 10), file(3, 1, 10)];
        // Angeheftet allein ist schon über dem Limit: alle freigebbaren gehen.
        assert_eq!(policy(50).plan(&files, NOW), vec![2, 3]);
    }

    #[test]
    fn zero_days_are_rejected() {
        assert_eq!(EvictionPolicy::new(0, 1), Err(DomainError::ZeroDays));
        assert_eq!(policy(7).unused_days(), 30);
        assert_eq!(policy(7).max_bytes(), 7);
    }
}

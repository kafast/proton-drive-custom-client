//! „Immer lokal halten“ und automatische Speicherfreigabe.

use crate::DomainError;
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

/// Regel für die automatische Freigabe lokal geladener Dateien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionPolicy {
    unused_days: u32,
}

/// Was über eine lokal geladene Datei bekannt ist, um über die Freigabe zu entscheiden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedFile {
    /// Angeheftet oder nicht.
    pub pin: PinState,
    /// Letzter Zugriff.
    pub last_access: Timestamp,
    /// `true`, wenn die entfernte Version vollständig und aktuell ist
    /// (keine ausstehenden Uploads, kein offener Konflikt).
    pub remote_is_current: bool,
}

impl EvictionPolicy {
    /// Dateien werden freigegeben, wenn sie `days` Tage nicht benutzt wurden.
    ///
    /// # Errors
    /// [`DomainError::ZeroDays`] bei `0`.
    pub const fn after_unused_days(days: u32) -> Result<Self, DomainError> {
        if days == 0 {
            return Err(DomainError::ZeroDays);
        }
        Ok(Self { unused_days: days })
    }

    /// Darf die lokale Kopie jetzt entfernt werden (nur Platzhalter bleibt)?
    ///
    /// Nie bei angehefteten Dateien und nie, solange die entfernte Version nicht
    /// aktuell ist; sonst ginge die einzige aktuelle Kopie verloren.
    #[must_use]
    pub fn may_evict(self, file: CachedFile, now: Timestamp) -> bool {
        file.pin == PinState::Unpinned
            && file.remote_is_current
            && now >= file.last_access.plus_days(self.unused_days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SECONDS_PER_DAY;

    const LONG_AGO: Timestamp = Timestamp::from_unix_seconds(0);
    const NOW: Timestamp = Timestamp::from_unix_seconds(10 * SECONDS_PER_DAY);

    fn policy() -> EvictionPolicy {
        EvictionPolicy::after_unused_days(7).unwrap()
    }

    fn file(pin: PinState, remote_is_current: bool) -> CachedFile {
        CachedFile {
            pin,
            last_access: LONG_AGO,
            remote_is_current,
        }
    }

    #[test]
    fn evicts_old_unpinned_synced_file() {
        assert!(policy().may_evict(file(PinState::Unpinned, true), NOW));
    }

    #[test]
    fn never_evicts_pinned_file() {
        assert!(!policy().may_evict(file(PinState::Pinned, true), NOW));
    }

    #[test]
    fn never_evicts_when_remote_is_not_current() {
        assert!(!policy().may_evict(file(PinState::Unpinned, false), NOW));
    }

    #[test]
    fn keeps_recently_used_file() {
        let recent = CachedFile {
            last_access: Timestamp::from_unix_seconds(9 * SECONDS_PER_DAY),
            ..file(PinState::Unpinned, true)
        };
        assert!(!policy().may_evict(recent, NOW));
    }

    #[test]
    fn zero_days_are_rejected() {
        assert_eq!(
            EvictionPolicy::after_unused_days(0),
            Err(DomainError::ZeroDays)
        );
    }
}

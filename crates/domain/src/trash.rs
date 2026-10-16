//! Papierkorb: Löschungen werden erst nach einer Frist endgültig.

use crate::DomainError;
use crate::node::NodeId;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Standard-Aufbewahrung im Papierkorb in Tagen.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Wie lange Einträge im Papierkorb bleiben (mindestens ein Tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPeriod {
    days: u32,
}

impl RetentionPeriod {
    /// Erzeugt eine Frist in Tagen.
    ///
    /// # Errors
    /// [`DomainError::ZeroDays`] bei `0`.
    pub const fn days(days: u32) -> Result<Self, DomainError> {
        if days == 0 {
            return Err(DomainError::ZeroDays);
        }
        Ok(Self { days })
    }

    /// Anzahl Tage.
    #[must_use]
    pub const fn in_days(self) -> u32 {
        self.days
    }
}

impl Default for RetentionPeriod {
    fn default() -> Self {
        Self {
            days: DEFAULT_RETENTION_DAYS,
        }
    }
}

/// Auf welcher Seite gelöscht wurde und wo der Inhalt jetzt liegt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOrigin {
    /// Lag lokal; der Inhalt liegt im lokalen Papierkorb unter `stored_as`.
    Local {
        /// Schlüssel im lokalen Papierkorb (vom Adapter vergeben).
        stored_as: String,
    },
    /// Lag in Proton; der Knoten liegt im Proton-Papierkorb.
    Remote {
        /// Kennung des Knotens in Proton.
        node: NodeId,
    },
}

/// Ein Eintrag im Papierkorb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// Ursprünglicher Pfad relativ zum Sync-Paar.
    pub original_path: RelativePath,
    /// Wo die Datei lag.
    pub origin: TrashOrigin,
    /// Wann sie in den Papierkorb kam.
    pub trashed_at: Timestamp,
}

impl TrashEntry {
    /// Ab wann der Eintrag endgültig gelöscht werden darf.
    #[must_use]
    pub const fn expires_at(&self, retention: RetentionPeriod) -> Timestamp {
        self.trashed_at.plus_days(retention.in_days())
    }

    /// `true`, wenn die Frist zum Zeitpunkt `now` abgelaufen ist.
    #[must_use]
    pub fn is_expired(&self, retention: RetentionPeriod, now: Timestamp) -> bool {
        now >= self.expires_at(retention)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SECONDS_PER_DAY;

    fn entry_at(seconds: i64) -> TrashEntry {
        TrashEntry {
            original_path: RelativePath::parse("a.txt").unwrap(),
            origin: TrashOrigin::Local {
                stored_as: "1".into(),
            },
            trashed_at: Timestamp::from_unix_seconds(seconds),
        }
    }

    #[test]
    fn default_retention_is_thirty_days() {
        assert_eq!(RetentionPeriod::default().in_days(), 30);
    }

    #[test]
    fn zero_days_are_rejected() {
        assert_eq!(RetentionPeriod::days(0), Err(DomainError::ZeroDays));
    }

    #[test]
    fn expires_exactly_after_retention() {
        let retention = RetentionPeriod::days(2).unwrap();
        let entry = entry_at(0);
        let just_before = Timestamp::from_unix_seconds(2 * SECONDS_PER_DAY - 1);
        let exactly = Timestamp::from_unix_seconds(2 * SECONDS_PER_DAY);
        assert!(!entry.is_expired(retention, just_before));
        assert!(entry.is_expired(retention, exactly));
    }
}

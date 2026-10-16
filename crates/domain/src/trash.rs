//! Trash: deletions only become final after a retention period.

use crate::DomainError;
use crate::node::NodeId;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Default trash retention in days.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// How long entries stay in the trash (at least one day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPeriod {
    days: u32,
}

impl RetentionPeriod {
    /// Creates a period in days.
    ///
    /// # Errors
    /// [`DomainError::ZeroDays`] for `0`.
    pub const fn days(days: u32) -> Result<Self, DomainError> {
        if days == 0 {
            return Err(DomainError::ZeroDays);
        }
        Ok(Self { days })
    }

    /// Number of days.
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

/// On which side the deletion happened and where the content is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOrigin {
    /// Was local; the content is in the local trash under `stored_as`.
    Local {
        /// Key in the local trash (assigned by the adapter).
        stored_as: String,
    },
    /// Was in Proton; the node is in the Proton trash.
    Remote {
        /// Id of the node in Proton.
        node: NodeId,
    },
}

/// An entry in the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// Original path relative to the sync pair.
    pub original_path: RelativePath,
    /// Where the file was.
    pub origin: TrashOrigin,
    /// When it was moved to the trash.
    pub trashed_at: Timestamp,
}

impl TrashEntry {
    /// From when on the entry may be deleted permanently.
    #[must_use]
    pub const fn expires_at(&self, retention: RetentionPeriod) -> Timestamp {
        self.trashed_at.plus_days(retention.in_days())
    }

    /// `true` if the period has expired at time `now`.
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

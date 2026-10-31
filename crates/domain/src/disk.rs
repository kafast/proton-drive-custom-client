//! Free space on the local disks: when is a disk "almost full" or "full"?

use crate::DomainError;

/// Default: below this share of free space a disk counts as almost full.
pub const DEFAULT_LOW_FREE_PERCENT: u8 = 10;
/// Default: below this share of free space a disk counts as full.
pub const DEFAULT_FULL_FREE_PERCENT: u8 = 2;
const PERCENT: u128 = 100;

/// Size and free space of the disk (file system) a folder lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    /// Identifies the file system, so that folders on the same disk are
    /// reported together; `None` if unknown.
    pub volume: Option<u64>,
    /// Bytes this user can still write.
    pub available_bytes: u64,
    /// Size of the file system in bytes.
    pub total_bytes: u64,
}

impl DiskSpace {
    /// Free space in percent of the size (rounded down); 0 for an empty size.
    #[must_use]
    pub fn free_percent(&self) -> u8 {
        if self.total_bytes == 0 {
            return 0;
        }
        let percent = u128::from(self.available_bytes) * PERCENT / u128::from(self.total_bytes);
        u8::try_from(percent.min(PERCENT)).unwrap_or(u8::MAX)
    }
}

/// How full a disk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiskLevel {
    /// Enough free space.
    Ok,
    /// Almost full: free space below the warning threshold.
    Low,
    /// Full: free space below the critical threshold; downloads will fail soon.
    Full,
}

/// Thresholds in percent of free space (`0 < full < low < 100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskThresholds {
    low_free_percent: u8,
    full_free_percent: u8,
}

impl DiskThresholds {
    /// Creates the thresholds.
    ///
    /// # Errors
    /// [`DomainError::InvalidDiskThresholds`] unless `0 < full < low < 100`.
    pub const fn new(low_free_percent: u8, full_free_percent: u8) -> Result<Self, DomainError> {
        if full_free_percent == 0
            || full_free_percent >= low_free_percent
            || low_free_percent >= 100
        {
            return Err(DomainError::InvalidDiskThresholds);
        }
        Ok(Self {
            low_free_percent,
            full_free_percent,
        })
    }

    /// Warning threshold in percent.
    #[must_use]
    pub const fn low_free_percent(&self) -> u8 {
        self.low_free_percent
    }

    /// Critical threshold in percent.
    #[must_use]
    pub const fn full_free_percent(&self) -> u8 {
        self.full_free_percent
    }

    /// The level of a disk with `space`.
    #[must_use]
    pub fn level(&self, space: &DiskSpace) -> DiskLevel {
        let free = space.free_percent();
        if free < self.full_free_percent {
            DiskLevel::Full
        } else if free < self.low_free_percent {
            DiskLevel::Low
        } else {
            DiskLevel::Ok
        }
    }
}

impl Default for DiskThresholds {
    fn default() -> Self {
        Self {
            low_free_percent: DEFAULT_LOW_FREE_PERCENT,
            full_free_percent: DEFAULT_FULL_FREE_PERCENT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(available_bytes: u64, total_bytes: u64) -> DiskSpace {
        DiskSpace {
            volume: None,
            available_bytes,
            total_bytes,
        }
    }

    #[test]
    fn free_percent_rounds_down_and_handles_empty_disks() {
        assert_eq!(space(50, 100).free_percent(), 50);
        assert_eq!(space(199, 10_000).free_percent(), 1);
        assert_eq!(space(0, 0).free_percent(), 0);
        assert_eq!(space(u64::MAX, u64::MAX).free_percent(), 100);
    }

    #[test]
    fn levels_follow_the_thresholds() {
        let thresholds = DiskThresholds::default();
        assert_eq!(thresholds.level(&space(10, 100)), DiskLevel::Ok);
        assert_eq!(thresholds.level(&space(9, 100)), DiskLevel::Low);
        assert_eq!(thresholds.level(&space(2, 100)), DiskLevel::Low);
        assert_eq!(thresholds.level(&space(1, 100)), DiskLevel::Full);
        assert_eq!(thresholds.level(&space(0, 0)), DiskLevel::Full);
    }

    #[test]
    fn thresholds_must_be_ordered() {
        assert!(DiskThresholds::new(20, 5).is_ok());
        for (low, full) in [(5, 5), (5, 10), (10, 0), (100, 2)] {
            assert_eq!(
                DiskThresholds::new(low, full),
                Err(DomainError::InvalidDiskThresholds),
                "{low}/{full}"
            );
        }
    }
}

//! Points in time and durations in whole seconds.
//!
//! Deliberately without an external time library: the domain only needs comparisons and
//! simple arithmetic. The current time is provided by the `Clock` port.

/// Seconds per day, for periods in days.
pub const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// A point in time as seconds since the Unix epoch (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Creates a point in time from Unix seconds.
    #[must_use]
    pub const fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }

    /// Returns the Unix seconds.
    #[must_use]
    pub const fn unix_seconds(self) -> i64 {
        self.0
    }

    /// The point in time `days` days later. Does not overflow but stays at the maximum.
    #[must_use]
    pub const fn plus_days(self, days: u32) -> Self {
        // `as i64` is lossless here because a u32 always fits into an i64.
        Self(self.0.saturating_add(days as i64 * SECONDS_PER_DAY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_days_adds_whole_days() {
        let start = Timestamp::from_unix_seconds(1_000);
        assert_eq!(
            start.plus_days(2).unix_seconds(),
            1_000 + 2 * SECONDS_PER_DAY
        );
    }

    #[test]
    fn plus_days_saturates_instead_of_overflowing() {
        let late = Timestamp::from_unix_seconds(i64::MAX - 1);
        assert_eq!(late.plus_days(1).unix_seconds(), i64::MAX);
    }
}

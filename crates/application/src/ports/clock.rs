//! Port for the current time, so that deadlines can be controlled in tests.

use std::time::{SystemTime, UNIX_EPOCH};

use protonsync_domain::job::LocalDateTime;
use protonsync_domain::time::Timestamp;

/// Returns the current time.
///
/// `Send + Sync` means: the object may be shared between threads,
/// which the asynchronous daemon needs.
pub trait Clock: Send + Sync {
    /// Current point in time.
    fn now(&self) -> Timestamp;
}

/// Real system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        // A system time before 1970 is a configuration error; then 0 applies.
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Timestamp::from_unix_seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
    }
}

/// A shared clock (`Arc`) is itself a clock.
impl<T: Clock + ?Sized> Clock for std::sync::Arc<T> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

/// Converts points in time to calendar time (e.g. for the monthly photo folders).
pub trait Calendar: Send + Sync {
    /// Calendar time of `at`; `None` outside the representable range.
    fn local(&self, at: Timestamp) -> Option<LocalDateTime>;
}

/// Calendar in UTC (for tests and as a fallback without a time zone).
#[derive(Debug, Clone, Copy, Default)]
pub struct UtcCalendar;

impl Calendar for UtcCalendar {
    fn local(&self, at: Timestamp) -> Option<LocalDateTime> {
        LocalDateTime::from_unix_utc(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_after_2025() {
        // 2025-01-01T00:00:00Z
        const START_OF_2025: i64 = 1_735_689_600;
        assert!(SystemClock.now().unix_seconds() > START_OF_2025);
    }
}

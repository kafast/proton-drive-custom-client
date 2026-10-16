use std::sync::atomic::{AtomicI64, Ordering};

use protonsync_application::ports::Clock;
use protonsync_domain::time::{SECONDS_PER_DAY, Timestamp};

/// Uhr für Tests, die nur durch [`FixedClock::advance_days`] weiterläuft.
///
/// `AtomicI64` erlaubt Änderungen über eine geteilte Referenz (`&self`),
/// ohne `Mutex`; ähnlich `AtomicLong` in Java.
#[derive(Debug)]
pub struct FixedClock {
    seconds: AtomicI64,
}

impl FixedClock {
    /// Uhr, die auf `start` steht.
    #[must_use]
    pub fn at(start: Timestamp) -> Self {
        Self {
            seconds: AtomicI64::new(start.unix_seconds()),
        }
    }

    /// Stellt die Uhr um ganze Tage vor.
    pub fn advance_days(&self, days: i64) {
        self.seconds
            .fetch_add(days * SECONDS_PER_DAY, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_seconds(self.seconds.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_only_on_request() {
        let clock = FixedClock::at(Timestamp::from_unix_seconds(5));
        assert_eq!(clock.now().unix_seconds(), 5);
        clock.advance_days(1);
        assert_eq!(clock.now().unix_seconds(), 5 + SECONDS_PER_DAY);
    }
}

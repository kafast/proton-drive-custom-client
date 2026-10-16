//! Port für die aktuelle Uhrzeit, damit Fristen in Tests steuerbar sind.

use std::time::{SystemTime, UNIX_EPOCH};

use protonsync_domain::job::LocalDateTime;
use protonsync_domain::time::Timestamp;

/// Liefert die aktuelle Zeit.
///
/// `Send + Sync` bedeutet: Das Objekt darf zwischen Threads geteilt werden,
/// was für den asynchronen Daemon nötig ist.
pub trait Clock: Send + Sync {
    /// Aktueller Zeitpunkt.
    fn now(&self) -> Timestamp;
}

/// Echte Systemuhr.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        // Eine Systemzeit vor 1970 ist ein Konfigurationsfehler; dann gilt 0.
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Timestamp::from_unix_seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
    }
}

/// Eine geteilte Uhr (`Arc`) ist selbst eine Uhr.
impl<T: Clock + ?Sized> Clock for std::sync::Arc<T> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

/// Rechnet Zeitpunkte in Kalenderzeit um (z. B. für die Monatsordner der Fotos).
pub trait Calendar: Send + Sync {
    /// Kalenderzeit von `at`; `None` außerhalb des darstellbaren Bereichs.
    fn local(&self, at: Timestamp) -> Option<LocalDateTime>;
}

/// Kalender in UTC (für Tests und als Rückfall ohne Zeitzone).
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

//! Zeitpunkte und Zeitspannen in ganzen Sekunden.
//!
//! Bewusst ohne externe Zeitbibliothek: Die Domäne braucht nur Vergleiche und
//! einfache Arithmetik. Die aktuelle Uhrzeit liefert der Port `Clock`.

/// Sekunden pro Tag, für Fristen in Tagen.
pub const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// Ein Zeitpunkt als Sekunden seit der Unix-Epoche (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Erzeugt einen Zeitpunkt aus Unix-Sekunden.
    #[must_use]
    pub const fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }

    /// Liefert die Unix-Sekunden.
    #[must_use]
    pub const fn unix_seconds(self) -> i64 {
        self.0
    }

    /// Zeitpunkt `days` Tage später. Läuft nicht über, sondern bleibt am Maximum.
    #[must_use]
    pub const fn plus_days(self, days: u32) -> Self {
        // `as i64` ist hier verlustfrei, weil u32 immer in i64 passt.
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

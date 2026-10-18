//! Conversion between instants and local time for cron schedules.
//!
//! Cron expressions mean wall-clock time ("every day at 3 o'clock"). The
//! time zone comes from the system (`TZ` or `/etc/localtime`); in a container
//! set `TZ=Europe/Berlin` for this. Without it, UTC applies.

use jiff::Timestamp as JiffTimestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use protonsync_application::ports::Calendar;
use protonsync_domain::job::{CronSchedule, LocalDateTime};
use protonsync_domain::time::Timestamp;

/// A time zone for schedules.
#[derive(Debug, Clone)]
pub struct LocalZone(TimeZone);

impl LocalZone {
    /// The system time zone; UTC if it cannot be determined.
    #[must_use]
    pub fn system() -> Self {
        Self(TimeZone::try_system().unwrap_or(TimeZone::UTC))
    }

    /// UTC (for tests and as a fallback).
    #[must_use]
    pub fn utc() -> Self {
        Self(TimeZone::UTC)
    }

    /// Time zone from a POSIX rule such as `CET-1CEST,M3.5.0,M10.5.0/3` (for tests).
    ///
    /// # Errors
    /// A message if the rule is invalid.
    pub fn posix(rule: &str) -> Result<Self, String> {
        TimeZone::posix(rule).map(Self).map_err(|e| e.to_string())
    }

    /// The time zone itself (e.g. for display in the web interface).
    #[must_use]
    pub fn time_zone(&self) -> &TimeZone {
        &self.0
    }

    /// Name of the time zone for logs, e.g. `Europe/Berlin`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.0.iana_name().unwrap_or("local")
    }

    /// Local time of an instant, truncated to the minute.
    #[must_use]
    pub fn to_local(&self, at: Timestamp) -> Option<LocalDateTime> {
        let civil = JiffTimestamp::from_second(at.unix_seconds())
            .ok()?
            .to_zoned(self.0.clone())
            .datetime();
        LocalDateTime::new(
            i32::from(civil.year()),
            u8::try_from(civil.month()).ok()?,
            u8::try_from(civil.day()).ok()?,
            u8::try_from(civil.hour()).ok()?,
            u8::try_from(civil.minute()).ok()?,
        )
    }

    /// Instant of a local time.
    ///
    /// If the time falls into the gap of a DST change (e.g. 2:30 in March),
    /// the instant right after the gap applies; if it occurs twice in October,
    /// the first occurrence applies.
    #[must_use]
    pub fn to_timestamp(&self, local: LocalDateTime) -> Option<Timestamp> {
        let civil = DateTime::new(
            i16::try_from(local.year()).ok()?,
            i8::try_from(local.month()).ok()?,
            i8::try_from(local.day()).ok()?,
            i8::try_from(local.hour()).ok()?,
            i8::try_from(local.minute()).ok()?,
            0,
            0,
        )
        .ok()?;
        let zoned = self.0.to_ambiguous_zoned(civil).compatible().ok()?;
        Some(Timestamp::from_unix_seconds(zoned.timestamp().as_second()))
    }

    /// Next run of a schedule after `after` (strictly after).
    #[must_use]
    pub fn next_run(&self, schedule: &CronSchedule, after: Timestamp) -> Option<Timestamp> {
        let mut local = self.to_local(after)?;
        // A time in the repeated hour in October can end up before `after`
        // after conversion; then take the following one.
        loop {
            local = schedule.next_after(local)?;
            let at = self.to_timestamp(local)?;
            if at > after {
                return Some(at);
            }
        }
    }
}

/// The month folders of the photos follow the same local time as the schedules.
impl Calendar for LocalZone {
    fn local(&self, at: Timestamp) -> Option<LocalDateTime> {
        self.to_local(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BERLIN: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

    fn berlin() -> LocalZone {
        LocalZone::posix(BERLIN).unwrap()
    }

    fn local(y: i32, mo: u8, d: u8, h: u8, mi: u8) -> LocalDateTime {
        LocalDateTime::new(y, mo, d, h, mi).unwrap()
    }

    fn cron(expression: &str) -> CronSchedule {
        CronSchedule::parse(expression).unwrap()
    }

    #[test]
    fn converts_both_ways_in_winter_and_summer() {
        let zone = berlin();
        // 2026-01-15 12:00 UTC is 13:00 winter time.
        let winter = Timestamp::from_unix_seconds(1_768_478_400);
        assert_eq!(zone.to_local(winter), Some(local(2026, 1, 15, 13, 0)));
        assert_eq!(zone.to_timestamp(local(2026, 1, 15, 13, 0)), Some(winter));
        // 2026-07-15 12:00 UTC is 14:00 summer time.
        let summer = Timestamp::from_unix_seconds(1_784_116_800);
        assert_eq!(zone.to_local(summer), Some(local(2026, 7, 15, 14, 0)));
        assert_eq!(zone.to_timestamp(local(2026, 7, 15, 14, 0)), Some(summer));
    }

    #[test]
    fn daily_run_keeps_wall_clock_time_across_dst() {
        let zone = berlin();
        let schedule = cron("0 3 * * *");
        // 2026-03-28 03:00 CET → next run 2026-03-29 03:00 CEST: only 23 hours.
        let before = zone.to_timestamp(local(2026, 3, 28, 3, 0)).unwrap();
        let next = zone.next_run(&schedule, before).unwrap();
        assert_eq!(next.unix_seconds() - before.unix_seconds(), 23 * 3600);
        assert_eq!(zone.to_local(next), Some(local(2026, 3, 29, 3, 0)));
    }

    #[test]
    fn time_in_spring_gap_runs_right_after_it() {
        let zone = berlin();
        // 2026-03-29 02:30 does not exist; 03:30 CEST (= 01:30 UTC) applies.
        let at = zone.to_timestamp(local(2026, 3, 29, 2, 30)).unwrap();
        assert_eq!(zone.to_local(at), Some(local(2026, 3, 29, 3, 30)));
    }

    #[test]
    fn repeated_autumn_hour_runs_once() {
        let zone = berlin();
        let schedule = cron("30 2 * * *");
        // First 02:30 on 2026-10-25 (summer time) …
        let first = zone.to_timestamp(local(2026, 10, 25, 2, 30)).unwrap();
        // … then not again until the next day, not one hour later.
        let next = zone.next_run(&schedule, first).unwrap();
        assert_eq!(zone.to_local(next), Some(local(2026, 10, 26, 2, 30)));
    }

    #[test]
    fn utc_and_system_zone_work() {
        let zone = LocalZone::utc();
        let at = Timestamp::from_unix_seconds(60);
        assert_eq!(zone.to_local(at), Some(local(1970, 1, 1, 0, 1)));
        assert_eq!(
            zone.next_run(&cron("* * * * *"), at)
                .unwrap()
                .unix_seconds(),
            120
        );
        assert_ne!(LocalZone::system().name(), "");
        assert!(LocalZone::posix("not a valid value").is_err());
    }
}

//! Schedules in cron format ("minute hour day month weekday").
//!
//! Calculations use **local calendar time** ([`LocalDateTime`]) without a
//! time zone: the daemon converts from and to real points in time (including
//! daylight saving time). This keeps the domain free of I/O.
//!
//! Supported are `*`, numbers, ranges `a-b`, lists `a,b`, steps `*/n`
//! or `a-b/n`, English abbreviations (`jan`, `mon`, ...) as well as `@hourly`, `@daily`,
//! `@weekly`, `@monthly` and `@yearly`. As with classic cron: if day
//! **and** weekday are restricted, it is enough for one of them to match.

use std::fmt;

use crate::DomainError;
use crate::time::{SECONDS_PER_DAY, Timestamp};

const MINUTES_PER_HOUR: u8 = 60;
const HOURS_PER_DAY: u8 = 24;
const MONTHS_PER_YEAR: u8 = 12;
const DAYS_PER_WEEK: u8 = 7;
const LONGEST_MONTH: u8 = 31;
/// How far [`CronSchedule::next_after`] searches into the future (8 years, so that
/// February 29 is found reliably too).
const SEARCH_DAYS: u32 = 8 * 366;

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// Calendar date and time (to the minute), without a time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDateTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
}

impl LocalDateTime {
    /// Validates and creates a point in time; `None` for invalid values
    /// (e.g. April 31 or 24:00).
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Option<Self> {
        let valid = (1..=MONTHS_PER_YEAR).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month)
            && hour < HOURS_PER_DAY
            && minute < MINUTES_PER_HOUR;
        valid.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
        })
    }

    /// Calendar time of a point in time in UTC (truncated to the minute);
    /// `None` outside the representable range. For local time the
    /// daemon converts using the system time zone.
    #[must_use]
    pub fn from_unix_utc(at: Timestamp) -> Option<Self> {
        let seconds = at.unix_seconds();
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let minute_of_day = seconds.rem_euclid(SECONDS_PER_DAY) / 60;
        let (year, month, day) = civil_from_days(days)?;
        Self::new(
            year,
            month,
            day,
            u8::try_from(minute_of_day / 60).ok()?,
            u8::try_from(minute_of_day % 60).ok()?,
        )
    }

    /// Year.
    #[must_use]
    pub fn year(self) -> i32 {
        self.year
    }

    /// Month (1–12).
    #[must_use]
    pub fn month(self) -> u8 {
        self.month
    }

    /// Day of the month (1–31).
    #[must_use]
    pub fn day(self) -> u8 {
        self.day
    }

    /// Hour (0–23).
    #[must_use]
    pub fn hour(self) -> u8 {
        self.hour
    }

    /// Minute (0–59).
    #[must_use]
    pub fn minute(self) -> u8 {
        self.minute
    }

    /// Weekday, 0 = Sunday to 6 = Saturday.
    #[must_use]
    pub fn weekday(self) -> u8 {
        // Days since 1970-01-01 (a Thursday, hence 4).
        let days = days_from_civil(self.year, self.month, self.day);
        let weekday = (days + 4).rem_euclid(i64::from(DAYS_PER_WEEK));
        // `rem_euclid` always returns 0..7, which fits losslessly into a u8.
        u8::try_from(weekday).unwrap_or(0)
    }

    /// Midnight of the following day.
    fn next_day(self) -> Self {
        let (year, month, day) = if self.day < days_in_month(self.year, self.month) {
            (self.year, self.month, self.day + 1)
        } else if self.month < MONTHS_PER_YEAR {
            (self.year, self.month + 1, 1)
        } else {
            (self.year + 1, 1, 1)
        };
        Self {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
        }
    }

    /// The next full minute.
    fn next_minute(self) -> Self {
        if self.minute + 1 < MINUTES_PER_HOUR {
            Self {
                minute: self.minute + 1,
                ..self
            }
        } else if self.hour + 1 < HOURS_PER_DAY {
            Self {
                hour: self.hour + 1,
                minute: 0,
                ..self
            }
        } else {
            self.next_day()
        }
    }
}

impl fmt::Display for LocalDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => LONGEST_MONTH,
    }
}

/// Days since 1970-01-01 ("days from civil" algorithm by Howard Hinnant).
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Date for days since 1970-01-01 (inverse of [`days_from_civil`], also
/// after Howard Hinnant); `None` if the year does not fit into `i32`.
fn civil_from_days(days: i64) -> Option<(i32, u8, u8)> {
    let days = days.checked_add(719_468)?;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Some((
        i32::try_from(year).ok()?,
        u8::try_from(month).ok()?,
        u8::try_from(day).ok()?,
    ))
}

/// A schedule in cron format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    expression: String,
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    days_restricted: bool,
    weekdays_restricted: bool,
}

impl CronSchedule {
    /// Reads an expression such as `30 3 * * *` (daily at 3:30) or `@daily`.
    ///
    /// # Errors
    /// [`DomainError::InvalidCron`] with a reason.
    pub fn parse(expression: &str) -> Result<Self, DomainError> {
        let invalid = |reason: String| DomainError::InvalidCron {
            expression: expression.to_owned(),
            reason,
        };
        let trimmed = expression.trim();
        let expanded = match trimmed.to_ascii_lowercase().as_str() {
            "@hourly" => "0 * * * *".to_owned(),
            "@daily" | "@midnight" => "0 0 * * *".to_owned(),
            "@weekly" => "0 0 * * 0".to_owned(),
            "@monthly" => "0 0 1 * *".to_owned(),
            "@yearly" | "@annually" => "0 0 1 1 *".to_owned(),
            other if other.starts_with('@') => return Err(invalid("unknown alias".into())),
            other => other.to_owned(),
        };
        let fields: Vec<&str> = expanded.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(invalid(format!(
                "expected 5 fields (minute hour day month weekday), found {}",
                fields.len()
            )));
        };
        let field = |text: &str, spec: FieldSpec| parse_field(text, spec).map_err(invalid);
        // Sunday may also be written as 7.
        let weekdays = field(weekday, WEEKDAY)?;
        let weekdays = (weekdays & 0x7F) | u64::from(weekdays & (1 << 7) != 0);
        let schedule = Self {
            expression: trimmed.to_owned(),
            minutes: field(minute, MINUTE)?,
            hours: low_bits(field(hour, HOUR)?),
            days: low_bits(field(day, DAY)?),
            months: low_bits(field(month, MONTH)?),
            weekdays: low_bits(weekdays),
            days_restricted: !day.starts_with('*'),
            weekdays_restricted: !weekday.starts_with('*'),
        };
        if !schedule.weekdays_restricted && !schedule.any_valid_day() {
            return Err(invalid("no month has this day".into()));
        }
        Ok(schedule)
    }

    /// The expression as it was given.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// `true` if the schedule plans a run at this minute.
    #[must_use]
    pub fn matches(&self, at: LocalDateTime) -> bool {
        self.matches_day(at) && self.matches_time(at.hour, at.minute)
    }

    /// The first scheduled point in time **after** `after`; `None` if there is
    /// none in the foreseeable future.
    #[must_use]
    pub fn next_after(&self, after: LocalDateTime) -> Option<LocalDateTime> {
        let mut candidate = after.next_minute();
        for _ in 0..SEARCH_DAYS {
            if self.matches_day(candidate)
                && let Some((hour, minute)) = self.first_time_from(candidate.hour, candidate.minute)
            {
                return Some(LocalDateTime {
                    hour,
                    minute,
                    ..candidate
                });
            }
            candidate = candidate.next_day();
        }
        None
    }

    fn matches_day(&self, at: LocalDateTime) -> bool {
        if !bit(u64::from(self.months), at.month) {
            return false;
        }
        let day = bit(u64::from(self.days), at.day);
        let weekday = bit(u64::from(self.weekdays), at.weekday());
        match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => day || weekday,
            (true, false) => day,
            (false, true) => weekday,
            (false, false) => true,
        }
    }

    fn matches_time(&self, hour: u8, minute: u8) -> bool {
        bit(u64::from(self.hours), hour) && bit(self.minutes, minute)
    }

    /// First matching time of a day from `hour:minute` on.
    fn first_time_from(&self, hour: u8, minute: u8) -> Option<(u8, u8)> {
        (hour..HOURS_PER_DAY)
            .filter(|h| bit(u64::from(self.hours), *h))
            .find_map(|h| {
                let start = if h == hour { minute } else { 0 };
                (start..MINUTES_PER_HOUR)
                    .find(|m| bit(self.minutes, *m))
                    .map(|m| (h, m))
            })
    }

    /// Is there at least one month that has one of the allowed days?
    fn any_valid_day(&self) -> bool {
        (1..=MONTHS_PER_YEAR)
            .filter(|month| bit(u64::from(self.months), *month))
            .any(|month| {
                // February with 29 days, so that leap years count.
                let longest = days_in_month(2000, month);
                (1..=longest).any(|day| bit(u64::from(self.days), day))
            })
    }
}

impl fmt::Display for CronSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.expression)
    }
}

fn bit(mask: u64, position: u8) -> bool {
    position < 64 && mask & (1 << position) != 0
}

/// Narrows a bitmask to a smaller type; the fields are limited so that
/// nothing is lost.
fn low_bits<T: TryFrom<u64> + Default>(mask: u64) -> T {
    T::try_from(mask).unwrap_or_default()
}

/// Value range and names of a field.
#[derive(Clone, Copy)]
struct FieldSpec {
    name: &'static str,
    min: u8,
    max: u8,
    names: &'static [&'static str],
    /// Value of the first name (months start at 1, weekdays at 0).
    first_named: u8,
}

const MINUTE: FieldSpec = FieldSpec {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
    first_named: 0,
};
const HOUR: FieldSpec = FieldSpec {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
    first_named: 0,
};
const DAY: FieldSpec = FieldSpec {
    name: "day",
    min: 1,
    max: 31,
    names: &[],
    first_named: 0,
};
const MONTH: FieldSpec = FieldSpec {
    name: "month",
    min: 1,
    max: 12,
    names: &MONTH_NAMES,
    first_named: 1,
};
const WEEKDAY: FieldSpec = FieldSpec {
    name: "weekday",
    min: 0,
    max: 7,
    names: &WEEKDAY_NAMES,
    first_named: 0,
};

/// Reads a field as a bitmask (bit n set = value n allowed).
fn parse_field(text: &str, spec: FieldSpec) -> Result<u64, String> {
    let mut mask = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u8 = step
                    .parse()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(|| format!("{}: invalid step {step:?}", spec.name))?;
                (range, step)
            }
            None => (item, 1),
        };
        let (start, end) = if range == "*" {
            (spec.min, spec.max)
        } else if let Some((from, to)) = range.split_once('-') {
            (value(from, spec)?, value(to, spec)?)
        } else {
            let single = value(range, spec)?;
            // `5/10` means "every 10 starting at 5".
            let end = if step > 1 { spec.max } else { single };
            (single, end)
        };
        if start > end {
            return Err(format!("{}: range {range:?} is backwards", spec.name));
        }
        for position in (start..=end).step_by(usize::from(step)) {
            mask |= 1 << position;
        }
    }
    Ok(mask)
}

fn value(text: &str, spec: FieldSpec) -> Result<u8, String> {
    let lower = text.to_ascii_lowercase();
    let named = spec
        .names
        .iter()
        .position(|name| *name == lower)
        .and_then(|index| u8::try_from(index).ok())
        .map(|index| index + spec.first_named);
    let number = named
        .or_else(|| text.parse().ok())
        .ok_or_else(|| format!("{}: {text:?} is not a number", spec.name))?;
    if (spec.min..=spec.max).contains(&number) {
        Ok(number)
    } else {
        Err(format!(
            "{}: {number} is outside {}–{}",
            spec.name, spec.min, spec.max
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> LocalDateTime {
        LocalDateTime::new(year, month, day, hour, minute).unwrap()
    }

    #[test]
    fn utc_calendar_time_of_known_instants() {
        let utc = |seconds| LocalDateTime::from_unix_utc(Timestamp::from_unix_seconds(seconds));
        assert_eq!(utc(0), Some(at(1970, 1, 1, 0, 0)));
        // 2026-10-16T13:45:59Z
        assert_eq!(utc(1_792_158_359), Some(at(2026, 10, 16, 13, 45)));
        // 2024-02-29T23:59:00Z (leap day)
        assert_eq!(utc(1_709_251_140), Some(at(2024, 2, 29, 23, 59)));
        // Before 1970: 1969-12-31T23:59:00Z
        assert_eq!(utc(-60), Some(at(1969, 12, 31, 23, 59)));
        assert_eq!(utc(i64::MAX), None);
    }

    proptest! {
        #[test]
        fn utc_conversion_round_trips_through_days(days in -1_000_000i64..1_000_000) {
            let start = LocalDateTime::from_unix_utc(Timestamp::from_unix_seconds(days * SECONDS_PER_DAY));
            let start = start.unwrap();
            prop_assert_eq!(days_from_civil(start.year(), start.month(), start.day()), days);
            prop_assert_eq!((start.hour(), start.minute()), (0, 0));
        }
    }

    fn next(expression: &str, after: LocalDateTime) -> LocalDateTime {
        CronSchedule::parse(expression)
            .unwrap()
            .next_after(after)
            .unwrap()
    }

    #[test]
    fn daily_at_fixed_time() {
        assert_eq!(
            next("30 3 * * *", at(2026, 10, 16, 2, 0)),
            at(2026, 10, 16, 3, 30)
        );
        assert_eq!(
            next("30 3 * * *", at(2026, 10, 16, 3, 30)),
            at(2026, 10, 17, 3, 30)
        );
    }

    #[test]
    fn steps_lists_and_ranges() {
        assert_eq!(
            next("*/15 * * * *", at(2026, 1, 1, 0, 1)),
            at(2026, 1, 1, 0, 15)
        );
        assert_eq!(
            next("0 8-10,18 * * *", at(2026, 1, 1, 10, 0)),
            at(2026, 1, 1, 18, 0)
        );
        assert_eq!(
            next("5/20 * * * *", at(2026, 1, 1, 0, 6)),
            at(2026, 1, 1, 0, 25)
        );
    }

    #[test]
    fn month_end_and_year_end_roll_over() {
        assert_eq!(
            next("0 0 * * *", at(2026, 12, 31, 23, 59)),
            at(2027, 1, 1, 0, 0)
        );
        assert_eq!(
            next("0 12 31 * *", at(2026, 4, 1, 0, 0)),
            at(2026, 5, 31, 12, 0)
        );
    }

    #[test]
    fn leap_day_is_found() {
        assert_eq!(
            next("0 0 29 2 *", at(2026, 3, 1, 0, 0)),
            at(2028, 2, 29, 0, 0)
        );
    }

    #[test]
    fn weekdays_by_number_name_and_seven() {
        // 2026-10-16 is a Friday.
        assert_eq!(at(2026, 10, 16, 0, 0).weekday(), 5);
        assert_eq!(
            next("0 9 * * mon", at(2026, 10, 16, 0, 0)),
            at(2026, 10, 19, 9, 0)
        );
        assert_eq!(
            next("0 9 * * 7", at(2026, 10, 16, 0, 0)),
            at(2026, 10, 18, 9, 0)
        );
        assert_eq!(
            next("0 9 * * MON-FRI", at(2026, 10, 17, 0, 0)),
            at(2026, 10, 19, 9, 0)
        );
    }

    #[test]
    fn day_or_weekday_when_both_are_restricted() {
        // On the 1st of the month or on Mondays.
        assert_eq!(
            next("0 0 1 * mon", at(2026, 10, 16, 0, 0)),
            at(2026, 10, 19, 0, 0)
        );
        assert_eq!(
            next("0 0 1 * mon", at(2026, 10, 27, 0, 0)),
            at(2026, 11, 1, 0, 0)
        );
    }

    #[test]
    fn aliases() {
        assert_eq!(next("@daily", at(2026, 1, 1, 0, 0)), at(2026, 1, 2, 0, 0));
        assert_eq!(next("@monthly", at(2026, 1, 1, 0, 0)), at(2026, 2, 1, 0, 0));
        assert_eq!(next("@hourly", at(2026, 1, 1, 0, 0)), at(2026, 1, 1, 1, 0));
        assert_eq!(
            next("@weekly", at(2026, 10, 16, 0, 0)),
            at(2026, 10, 18, 0, 0)
        );
    }

    #[test]
    fn invalid_expressions_are_rejected_with_reason() {
        for bad in [
            "",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "x * * * *",
            "0 0 30 2 *",
            "@sometimes",
        ] {
            assert!(
                matches!(
                    CronSchedule::parse(bad),
                    Err(DomainError::InvalidCron { .. })
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn local_date_time_validates() {
        assert!(LocalDateTime::new(2026, 4, 31, 0, 0).is_none());
        assert!(LocalDateTime::new(2026, 2, 29, 0, 0).is_none());
        assert!(LocalDateTime::new(2028, 2, 29, 0, 0).is_some());
        assert!(LocalDateTime::new(2026, 1, 1, 24, 0).is_none());
        assert_eq!(at(2026, 1, 2, 3, 4).to_string(), "2026-01-02 03:04");
    }

    fn any_time() -> impl Strategy<Value = LocalDateTime> {
        (2000i32..2100, 1u8..=12, 1u8..=28, 0u8..24, 0u8..60)
            .prop_map(|(y, mo, d, h, mi)| at(y, mo, d, h, mi))
    }

    proptest! {
        /// The result lies after it, matches the schedule, and no minute in between matches.
        #[test]
        fn next_after_is_the_first_match(
            start in any_time(),
            minute in 0u8..60,
            hour in prop::option::of(0u8..24),
            weekday in prop::option::of(0u8..7),
        ) {
            let expression = format!(
                "{minute} {} * * {}",
                hour.map_or("*".to_owned(), |h| h.to_string()),
                weekday.map_or("*".to_owned(), |w| w.to_string()),
            );
            let schedule = CronSchedule::parse(&expression).unwrap();
            let found = schedule.next_after(start).unwrap();
            prop_assert!(found > start);
            prop_assert!(schedule.matches(found));
            let mut probe = start.next_minute();
            while probe < found {
                prop_assert!(!schedule.matches(probe), "{probe} matches too");
                probe = probe.next_minute();
            }
        }
    }
}

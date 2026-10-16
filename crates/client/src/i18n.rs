//! Bilingual output (German/English), date and size format.

/// Output language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// German.
    De,
    /// English (the default when the environment says nothing else).
    En,
}

impl Lang {
    /// Determines the language the usual way from `LC_ALL`, `LC_MESSAGES` and `LANG`.
    #[must_use]
    pub fn from_env() -> Self {
        let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
        Self::from_locale(value.as_deref())
    }

    /// Language from a locale name such as `de_DE.UTF-8`.
    #[must_use]
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(locale) if locale.starts_with("de") => Self::De,
            _ => Self::En,
        }
    }

    /// Picks the text in this language.
    #[must_use]
    pub fn pick<'a>(self, de: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::De => de,
            Self::En => en,
        }
    }
}

const BYTES_PER_UNIT: f64 = 1000.0;
const BYTE_UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// Bytes in readable form, e.g. `1.5 GB` (decimal units as in `[cache] max_size_gb`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    // The precision of f64 is plenty for display.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= BYTES_PER_UNIT && unit + 1 < BYTE_UNITS.len() {
        value /= BYTES_PER_UNIT;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", BYTE_UNITS[unit])
    }
}

const SECONDS_PER_DAY: i64 = 86_400;
const DAYS_PER_ERA: i64 = 146_097;
const DAYS_FROM_YEAR_0_TO_1970: i64 = 719_468;

/// Unix seconds as a date `YYYY-MM-DD` (UTC).
#[must_use]
pub fn format_date(unix_seconds: i64) -> String {
    // The "civil_from_days" algorithm (Howard Hinnant), so no date library is needed.
    let days = unix_seconds.div_euclid(SECONDS_PER_DAY) + DAYS_FROM_YEAR_0_TO_1970;
    let era = days.div_euclid(DAYS_PER_ERA);
    let day_of_era = days - era * DAYS_PER_ERA;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Unix seconds as date and time `YYYY-MM-DD HH:MM` in the time zone `zone`.
#[must_use]
pub fn format_date_time(unix_seconds: i64, zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(unix_seconds).map_or_else(
        |_| format_date(unix_seconds),
        |at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_german_locales() {
        assert_eq!(Lang::from_locale(Some("de_DE.UTF-8")), Lang::De);
        assert_eq!(Lang::from_locale(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(Lang::from_locale(Some("C")), Lang::En);
        assert_eq!(Lang::from_locale(None), Lang::En);
        assert_eq!(Lang::De.pick("Ja", "Yes"), "Ja");
    }

    #[test]
    fn bytes_are_readable() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 kB");
        assert_eq!(format_bytes(10_000_000_000), "10.0 GB");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(951_782_400), "2000-02-29");
        assert_eq!(format_date(1_760_000_000), "2025-10-09");
        assert_eq!(format_date(-1), "1969-12-31");
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(format_date_time(1_760_000_000, &utc), "2025-10-09 08:53");
        let berlin = jiff::tz::TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(format_date_time(1_760_000_000, &berlin), "2025-10-09 10:53");
    }
}

//! Zweisprachige Ausgabe (Deutsch/Englisch) und Datumsformat.

/// Sprache der Ausgabe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Deutsch.
    De,
    /// Englisch (Standard, wenn die Umgebung nichts anderes sagt).
    En,
}

impl Lang {
    /// Ermittelt die Sprache wie üblich aus `LC_ALL`, `LC_MESSAGES` und `LANG`.
    #[must_use]
    pub fn from_env() -> Self {
        let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
        Self::from_locale(value.as_deref())
    }

    /// Sprache aus einem Locale-Namen wie `de_DE.UTF-8`.
    #[must_use]
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(locale) if locale.starts_with("de") => Self::De,
            _ => Self::En,
        }
    }

    /// Wählt den Text in der eigenen Sprache.
    #[must_use]
    pub fn pick<'a>(self, de: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::De => de,
            Self::En => en,
        }
    }
}

const SECONDS_PER_DAY: i64 = 86_400;
const DAYS_PER_ERA: i64 = 146_097;
const DAYS_FROM_YEAR_0_TO_1970: i64 = 719_468;

/// Unix-Sekunden als Datum `JJJJ-MM-TT` (UTC).
#[must_use]
pub fn format_date(unix_seconds: i64) -> String {
    // Verfahren „civil_from_days“ (Howard Hinnant), damit keine Datums-Bibliothek nötig ist.
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

/// Unix-Sekunden als Datum und Uhrzeit `JJJJ-MM-TT HH:MM` in der Zeitzone `zone`.
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

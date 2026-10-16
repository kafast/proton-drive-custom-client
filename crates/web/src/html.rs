//! HTML-Bausteine: Sprache, Escaping, Seitenrahmen. Bewusst ohne Template-Engine
//! und ohne JavaScript, damit die Oberfläche klein und leicht prüfbar bleibt.

use std::fmt::Write;

/// Sprache der Oberfläche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Deutsch.
    De,
    /// Englisch.
    En,
}

impl Lang {
    /// Wählt den Text in dieser Sprache.
    #[must_use]
    pub fn pick<'a>(self, de: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::De => de,
            Self::En => en,
        }
    }

    /// Kürzel für `lang`-Attribut und Cookie.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    /// Liest ein Kürzel (`de`, `en`).
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "de" => Some(Self::De),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    /// Sprache aus dem `Accept-Language`-Header: Deutsch, wenn Deutsch zuerst kommt.
    #[must_use]
    pub fn from_accept_language(header: Option<&str>) -> Self {
        let first = header
            .and_then(|h| h.split(',').next())
            .map(|tag| tag.trim().to_ascii_lowercase());
        match first {
            Some(tag) if tag.starts_with("de") => Self::De,
            _ => Self::En,
        }
    }
}

/// Maskiert Text für HTML (Inhalt und Attribute in doppelten Anführungszeichen).
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Unix-Sekunden als `JJJJ-MM-TT HH:MM` in der Zeitzone `zone`.
#[must_use]
pub fn format_time(unix_seconds: i64, zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(unix_seconds).map_or_else(
        |_| unix_seconds.to_string(),
        |at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

/// Ein Menüpunkt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// Übersicht.
    Status,
    /// Konflikte.
    Conflicts,
    /// Papierkorb.
    Trash,
    /// Upload-Jobs.
    Jobs,
    /// Einstellungen.
    Settings,
    /// Keiner (z. B. Anmeldeseite).
    None,
}

/// Rahmen einer Seite. `csrf` ist nur bei angemeldeten Seiten gesetzt.
#[must_use]
pub fn page(lang: Lang, nav: Nav, title: &str, csrf: Option<&str>, body: &str) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>protonsync – {}</title><link rel=\"stylesheet\" href=\"/static/style.css\">\
         </head><body><header><strong>protonsync</strong>",
        lang.code(),
        escape(title)
    );
    if let Some(csrf) = csrf {
        out.push_str("<nav>");
        for (item, href, de, en) in [
            (Nav::Status, "/", "Übersicht", "Overview"),
            (Nav::Conflicts, "/conflicts", "Konflikte", "Conflicts"),
            (Nav::Trash, "/trash", "Papierkorb", "Trash"),
            (Nav::Jobs, "/jobs", "Upload-Jobs", "Upload jobs"),
            (Nav::Settings, "/settings", "Einstellungen", "Settings"),
        ] {
            let current = if item == nav {
                " aria-current=\"page\""
            } else {
                ""
            };
            let _ = write!(out, "<a href=\"{href}\"{current}>{}</a>", lang.pick(de, en));
        }
        let _ = write!(
            out,
            "</nav><form method=\"post\" action=\"/logout\">{}<button>{}</button></form>",
            csrf_field(csrf),
            lang.pick("Abmelden", "Log out")
        );
    }
    let other = match lang {
        Lang::De => ("en", "English"),
        Lang::En => ("de", "Deutsch"),
    };
    let _ = writeln!(
        out,
        "<a class=\"lang\" href=\"?lang={}\">{}</a></header><main><h1>{}</h1>{body}</main>\
         <footer>{}</footer></body></html>",
        other.0,
        other.1,
        escape(title),
        lang.pick(
            "Inoffizieller Client, nicht von Proton entwickelt oder unterstützt.",
            "Unofficial client, not developed or supported by Proton."
        )
    );
    out
}

/// Verstecktes Formularfeld mit der CSRF-Kennung.
#[must_use]
pub fn csrf_field(csrf: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        escape(csrf)
    )
}

/// Ein Formular mit einem Knopf, das per POST an `action` geht.
#[must_use]
pub fn button_form(action: &str, csrf: &str, fields: &[(&str, &str)], label: &str) -> String {
    let mut out = format!(
        "<form class=\"inline\" method=\"post\" action=\"{}\">{}",
        escape(action),
        csrf_field(csrf)
    );
    for (name, value) in fields {
        let _ = write!(
            out,
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        );
    }
    let _ = write!(out, "<button>{}</button></form>", escape(label));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_special_characters() {
        assert_eq!(
            escape("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(escape("Grüße"), "Grüße");
    }

    #[test]
    fn language_from_header_and_code() {
        assert_eq!(
            Lang::from_accept_language(Some("de-DE,de;q=0.9,en;q=0.8")),
            Lang::De
        );
        assert_eq!(Lang::from_accept_language(Some("en-US,de;q=0.5")), Lang::En);
        assert_eq!(Lang::from_accept_language(None), Lang::En);
        assert_eq!(Lang::from_code("de"), Some(Lang::De));
        assert_eq!(Lang::from_code("fr"), None);
        assert_eq!(Lang::En.code(), "en");
    }

    #[test]
    fn formats_times_in_the_given_zone() {
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(format_time(0, &utc), "1970-01-01 00:00");
        assert_eq!(format_time(1_760_000_000, &utc), "2025-10-09 08:53");
        let berlin = jiff::tz::TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(format_time(1_760_000_000, &berlin), "2025-10-09 10:53");
    }

    #[test]
    fn pages_escape_titles_and_show_navigation_only_when_logged_in() {
        let anonymous = page(Lang::De, Nav::None, "<x>", None, "inhalt");
        assert!(anonymous.contains("<h1>&lt;x&gt;</h1>"));
        assert!(!anonymous.contains("<nav>"));
        let logged_in = page(Lang::En, Nav::Jobs, "Jobs", Some("t\"1"), "");
        assert!(logged_in.contains("<a href=\"/jobs\" aria-current=\"page\">Upload jobs</a>"));
        assert!(logged_in.contains("value=\"t&quot;1\""));
        assert!(logged_in.contains("?lang=de"));
    }

    #[test]
    fn button_forms_carry_csrf_and_fields() {
        let form = button_form(
            "/trash/restore",
            "c",
            &[("pair", "main"), ("id", "3")],
            "<ok>",
        );
        assert!(form.contains("action=\"/trash/restore\""));
        assert!(form.contains("name=\"csrf\" value=\"c\""));
        assert!(form.contains("name=\"pair\" value=\"main\""));
        assert!(form.contains("<button>&lt;ok&gt;</button>"));
    }
}

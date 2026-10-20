//! The commented example configuration (`protonsync config example`).
//!
//! The file `config.example.toml` lists every setting with its default and
//! bilingual (German/English) comments. The tests below keep it in step with
//! the code: every field the parser knows must appear in it, and every value
//! it shows as a default must really be the default.

/// The example configuration with all settings, defaults and comments.
pub const EXAMPLE_CONFIG: &str = include_str!("../config.example.toml");

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde::de::{self, Deserializer, Visitor};

    use super::EXAMPLE_CONFIG;
    use crate::config::{Config, Environment, RawConfig, RawGeneral, RawPair};
    use crate::job_config::RawJob;
    use crate::mount_config::{RawCache, RawMount};
    use crate::photo_config::RawPhotos;
    use crate::web_config::RawWeb;

    fn env() -> Environment {
        Environment {
            home: Some("/home/k".into()),
            ..Environment::default()
        }
    }

    fn parse(text: &str) -> Config {
        Config::parse(text, &env()).unwrap_or_else(|e| panic!("{e}\n---\n{text}"))
    }

    /// A template line is a commented-out setting or section header
    /// (`# key = value`, `# [web]`, `# [[job]]`); explanations start with an
    /// upper-case word or are indented.
    fn is_template_line(line: &str) -> bool {
        let Some(rest) = line.strip_prefix("# ") else {
            return false;
        };
        if rest.starts_with('[') {
            return rest.ends_with(']');
        }
        rest.split_once(" = ").is_some_and(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
    }

    /// The example with every template uncommented and the web interface
    /// switched on (with a real password hash), so that all values get parsed.
    fn everything_on() -> toml::Table {
        let text: String = EXAMPLE_CONFIG
            .lines()
            .map(|line| {
                if is_template_line(line) {
                    &line[2..]
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut table: toml::Table = text.parse().unwrap();
        let web = table["web"].as_table_mut().unwrap();
        web.insert("enabled".into(), true.into());
        let hash = protonsync_web::hash_password("secret").unwrap();
        web.insert("password_hash".into(), hash.into());
        table
    }

    fn section<'a>(table: &'a mut toml::Table, name: &str) -> &'a mut toml::Table {
        match table.get_mut(name) {
            Some(toml::Value::Array(entries)) => entries[0].as_table_mut().unwrap(),
            Some(toml::Value::Table(section)) => section,
            _ => panic!("section {name} is missing in the example"),
        }
    }

    /// Records the field names a `Deserialize` type asks for.
    struct FieldNames<'a>(&'a mut &'static [&'static str]);

    impl<'de> Deserializer<'de> for FieldNames<'_> {
        type Error = de::value::Error;

        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
            Err(de::Error::custom("not a struct"))
        }

        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _: &'static str,
            fields: &'static [&'static str],
            _: V,
        ) -> Result<V::Value, Self::Error> {
            *self.0 = fields;
            Err(de::Error::custom("fields recorded"))
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map
            enum identifier ignored_any
        }
    }

    fn fields_of<T: for<'de> Deserialize<'de>>() -> &'static [&'static str] {
        let mut fields: &'static [&'static str] = &[];
        // The error is expected: the deserializer stops once it knows the fields.
        let _ = T::deserialize(FieldNames(&mut fields));
        assert_ne!(fields.len(), 0, "not a struct");
        fields
    }

    #[test]
    fn the_example_as_shipped_means_the_defaults() {
        let shipped = parse(EXAMPLE_CONFIG);
        let defaults = parse("");
        assert!(shipped.dry_run);
        assert_eq!(shipped.data_dir, defaults.data_dir);
        assert_eq!(shipped.secrets, defaults.secrets);
        assert_eq!(shipped.retention, defaults.retention);
        assert_eq!(shipped.timing, defaults.timing);
        assert_eq!(shipped.cache, defaults.cache);
        assert_eq!(
            (shipped.web, shipped.photos, shipped.jobs, shipped.mounts),
            (None, None, Vec::new(), Vec::new())
        );
        assert_eq!(shipped.pairs.len(), 1, "one pair to adjust");
    }

    #[test]
    fn every_setting_is_documented() {
        let mut table = everything_on();
        let sections: [(&str, &[&str]); 7] = [
            ("general", fields_of::<RawGeneral>()),
            ("pair", fields_of::<RawPair>()),
            ("job", fields_of::<RawJob>()),
            ("web", fields_of::<RawWeb>()),
            ("mount", fields_of::<RawMount>()),
            ("cache", fields_of::<RawCache>()),
            ("photos", fields_of::<RawPhotos>()),
        ];
        let known: Vec<&str> = sections.iter().map(|(name, _)| *name).collect();
        assert_eq!(fields_of::<RawConfig>(), known.as_slice());
        for (name, fields) in sections {
            let section = section(&mut table, name);
            for field in fields {
                assert!(section.contains_key(*field), "[{name}] {field} is missing");
            }
        }
    }

    #[test]
    fn every_template_parses() {
        let config = parse(&everything_on().to_string());
        assert_eq!(config.jobs.len(), 1);
        assert_eq!(config.mounts.len(), 1);
        assert!(config.web.is_some() && config.photos.is_some());
    }

    #[test]
    fn values_shown_as_defaults_are_the_defaults() {
        // Leaving these keys out must not change anything.
        let defaults: [(&str, &[&str]); 7] = [
            (
                "general",
                &[
                    "dry_run",
                    "trash_retention_days",
                    "event_poll_seconds",
                    "full_sync_minutes",
                    "local_debounce_seconds",
                ],
            ),
            ("pair", &["direction"]),
            (
                "job",
                &["pattern", "verify", "max_attempts", "settle_seconds"],
            ),
            ("web", &["bind", "allowed_networks", "session_hours"]),
            ("mount", &["remote", "keep_local"]),
            ("cache", &["evict_after_days", "max_size_gb"]),
            ("photos", &["direction"]),
        ];
        let full = everything_on();
        let expected = parse(&full.to_string());
        for (name, keys) in defaults {
            let mut reduced = full.clone();
            let section = section(&mut reduced, name);
            for key in keys {
                assert!(section.remove(*key).is_some(), "[{name}] {key} is missing");
            }
            assert_eq!(
                parse(&reduced.to_string()),
                expected,
                "[{name}] shows a value that is not the default"
            );
        }
    }
}

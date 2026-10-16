//! Foto-Abgleich (`[photos]`) in der Konfigurationsdatei (ADR 0012).
//!
//! Beispiel:
//!
//! ```toml
//! [photos]
//! folder = "/home/krischan/Bilder/Proton"
//! direction = "download_only"   # Standard; oder "upload_only", "two_way"
//! ```
//!
//! Fehlt der Abschnitt, werden keine Fotos abgeglichen.

use std::path::PathBuf;

use protonsync_domain::sync_pair::SyncDirection;
use serde::Deserialize;

use crate::config::{ConfigError, RawDirection};

/// Einstellungen des Foto-Abgleichs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoConfig {
    /// Lokaler Fotoordner (absolut); Fotos liegen darin unter `JJJJ/MM`.
    pub folder: PathBuf,
    /// Richtung; Standard nur herunterladen (Sicherung der Handy-Fotos).
    pub direction: SyncDirection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPhotos {
    folder: Option<PathBuf>,
    direction: Option<RawDirection>,
}

impl RawPhotos {
    /// Die Einstellungen; `None`, wenn der Abschnitt fehlt.
    pub(crate) fn into_config(self) -> Result<Option<PhotoConfig>, ConfigError> {
        let Some(folder) = self.folder else {
            if self.direction.is_some() {
                return Err(ConfigError::Invalid("[photos]: folder fehlt".into()));
            }
            return Ok(None);
        };
        if !folder.is_absolute() {
            return Err(ConfigError::Invalid(
                "[photos]: folder muss ein absoluter Pfad sein".into(),
            ));
        }
        Ok(Some(PhotoConfig {
            folder,
            direction: self
                .direction
                .map_or(SyncDirection::DownloadOnly, SyncDirection::from),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Environment};

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, &Environment::default())
    }

    #[test]
    fn photos_are_off_unless_configured() {
        assert_eq!(parse("").unwrap().photos, None);
    }

    #[test]
    fn photos_default_to_download_only() {
        let config = parse("[photos]\nfolder = \"/home/k/Bilder\"").unwrap();
        assert_eq!(
            config.photos,
            Some(PhotoConfig {
                folder: "/home/k/Bilder".into(),
                direction: SyncDirection::DownloadOnly,
            })
        );
        let two_way = parse("[photos]\nfolder = \"/b\"\ndirection = \"two_way\"").unwrap();
        assert_eq!(two_way.photos.unwrap().direction, SyncDirection::TwoWay);
    }

    #[test]
    fn invalid_photo_settings_are_rejected() {
        for text in [
            "[photos]\nfolder = \"relativ\"",
            "[photos]\ndirection = \"two_way\"",
            "[photos]\nfolder = \"/b\"\nunbekannt = 1",
            "[[pair]]\nid = \"a\"\nlocal = \"/home/k\"\n[photos]\nfolder = \"/home/k/Bilder\"",
            "[[mount]]\nid = \"m\"\nmountpoint = \"/m\"\n[photos]\nfolder = \"/m\"",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}

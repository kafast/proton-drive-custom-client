//! Photo sync (`[photos]`) in the configuration file (ADR 0012).
//!
//! Example:
//!
//! ```toml
//! [photos]
//! folder = "/home/krischan/Pictures/Proton"
//! direction = "download_only"   # default; or "upload_only", "two_way"
//! ```
//!
//! Without the section, no photos are synced.

use std::path::PathBuf;

use protonsync_domain::sync_pair::SyncDirection;
use serde::Deserialize;

use crate::config::{ConfigError, RawDirection};

/// Settings of the photo sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoConfig {
    /// Local photo folder (absolute); photos are stored in it under `YYYY/MM`.
    pub folder: PathBuf,
    /// Direction; default download only (backup of the phone photos).
    pub direction: SyncDirection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPhotos {
    folder: Option<PathBuf>,
    direction: Option<RawDirection>,
}

impl RawPhotos {
    /// The settings; `None` if the section is missing.
    pub(crate) fn into_config(self) -> Result<Option<PhotoConfig>, ConfigError> {
        let Some(folder) = self.folder else {
            if self.direction.is_some() {
                return Err(ConfigError::Invalid("[photos]: folder is missing".into()));
            }
            return Ok(None);
        };
        if !folder.is_absolute() {
            return Err(ConfigError::Invalid(
                "[photos]: folder must be an absolute path".into(),
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
        let config = parse("[photos]\nfolder = \"/home/k/Pictures\"").unwrap();
        assert_eq!(
            config.photos,
            Some(PhotoConfig {
                folder: "/home/k/Pictures".into(),
                direction: SyncDirection::DownloadOnly,
            })
        );
        let two_way = parse("[photos]\nfolder = \"/b\"\ndirection = \"two_way\"").unwrap();
        assert_eq!(two_way.photos.unwrap().direction, SyncDirection::TwoWay);
    }

    #[test]
    fn invalid_photo_settings_are_rejected() {
        for text in [
            "[photos]\nfolder = \"relative\"",
            "[photos]\ndirection = \"two_way\"",
            "[photos]\nfolder = \"/b\"\nunknown = 1",
            "[[pair]]\nid = \"a\"\nlocal = \"/home/k\"\n[photos]\nfolder = \"/home/k/Pictures\"",
            "[[mount]]\nid = \"m\"\nmountpoint = \"/m\"\n[photos]\nfolder = \"/m\"",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}

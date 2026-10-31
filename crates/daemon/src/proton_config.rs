//! Choice of the Proton connection (`[proton]`, ADR 0025).
//!
//! Example:
//!
//! ```toml
//! [proton]
//! backend = "official"     # default: Proton's official SDK through a helper process
//!                          # "community": the Rust community crates (ADR 0002)
//! helper = "/usr/lib/protonsync/protonsync-sdk-helper"  # optional
//! ```
//!
//! With `official`, sign-in happens in the browser (`protonsync login`
//! shows a link); with `community`, with password and 2FA code.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::ConfigError;

/// Name of the helper program.
pub const HELPER_PROGRAM: &str = "protonsync-sdk-helper";

/// Where packages install the helper.
const PACKAGED_HELPER: &str = "/usr/lib/protonsync/protonsync-sdk-helper";

/// Which implementation talks to Proton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtonBackend {
    /// Proton's official TypeScript SDK through a helper process (default).
    Official,
    /// The Rust community crates `proton-sdk`/`proton-drive-rs`.
    Community,
}

/// Settings of the Proton connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonConfig {
    /// Chosen implementation.
    pub backend: ProtonBackend,
    /// Helper program; `None` searches next to the daemon, then in the package path.
    pub helper: Option<PathBuf>,
}

impl Default for ProtonConfig {
    fn default() -> Self {
        Self {
            backend: ProtonBackend::Official,
            helper: None,
        }
    }
}

impl ProtonConfig {
    /// The helper program to start: the configured one, else the one next to
    /// `daemon_exe`, else the package path.
    #[must_use]
    pub fn helper_program(&self, daemon_exe: Option<&Path>) -> PathBuf {
        if let Some(helper) = &self.helper {
            return helper.clone();
        }
        daemon_exe
            .and_then(Path::parent)
            .map(|dir| dir.join(HELPER_PROGRAM))
            .filter(|candidate| candidate.is_file())
            .unwrap_or_else(|| PathBuf::from(PACKAGED_HELPER))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProton {
    backend: Option<ProtonBackend>,
    helper: Option<PathBuf>,
}

impl RawProton {
    pub(crate) fn into_config(self) -> Result<ProtonConfig, ConfigError> {
        if let Some(helper) = &self.helper
            && !helper.is_absolute()
        {
            return Err(ConfigError::Invalid(format!(
                "[proton] helper must be an absolute path: {}",
                helper.display()
            )));
        }
        Ok(ProtonConfig {
            backend: self.backend.unwrap_or(ProtonBackend::Official),
            helper: self.helper,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ProtonConfig, ConfigError> {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            proton: RawProton,
        }
        let wrapper: Wrapper = toml::from_str(text).unwrap();
        wrapper.proton.into_config()
    }

    #[test]
    fn official_is_the_default() {
        assert_eq!(parse("").unwrap(), ProtonConfig::default());
        assert_eq!(parse("").unwrap().backend, ProtonBackend::Official);
    }

    #[test]
    fn community_and_helper_path_are_read() {
        let config = parse("[proton]\nbackend = \"community\"\nhelper = \"/opt/h\"").unwrap();
        assert_eq!(config.backend, ProtonBackend::Community);
        assert_eq!(config.helper_program(None), PathBuf::from("/opt/h"));
    }

    #[test]
    fn relative_helper_paths_are_rejected() {
        assert!(parse("[proton]\nhelper = \"h\"").is_err());
    }

    #[test]
    fn helper_is_searched_next_to_the_daemon_then_in_the_package_path() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = dir.path().join("protonsyncd");
        let config = ProtonConfig::default();
        assert_eq!(
            config.helper_program(Some(&daemon)),
            PathBuf::from(PACKAGED_HELPER)
        );
        std::fs::write(dir.path().join(HELPER_PROGRAM), b"").unwrap();
        assert_eq!(
            config.helper_program(Some(&daemon)),
            dir.path().join(HELPER_PROGRAM)
        );
    }
}

//! The key the server uses to encrypt its secret file.

use std::path::{Path, PathBuf};

use thiserror::Error;
use zeroize::Zeroizing;

/// Name of the systemd credential (`LoadCredential=protonsync-key:…`).
pub const KEY_CREDENTIAL_NAME: &str = "protonsync-key";
/// Environment variable holding the path of a key file.
pub const KEY_FILE_ENV: &str = "PROTONSYNC_KEY_FILE";
/// Environment variable holding the key itself (e.g. as a container secret).
pub const KEY_ENV: &str = "PROTONSYNC_KEY";

/// Length of the key in bytes (AES-256).
const KEY_BYTES: usize = 32;
/// Permissions that may be set for group and others: none.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;

/// Error while obtaining the key.
#[derive(Debug, Error)]
pub enum KeyError {
    /// Neither credential nor file nor variable is set.
    #[error(
        "No key found: set systemd credential {KEY_CREDENTIAL_NAME}, {KEY_FILE_ENV} or {KEY_ENV}"
    )]
    Missing,
    /// The file is readable by others.
    #[error("key file {0} is readable by others; please run `chmod 600`")]
    TooOpen(PathBuf),
    /// The content is not a key of 64 hex characters.
    #[error("key must consist of 64 hex characters (e.g. `openssl rand -hex 32`)")]
    Malformed,
    /// The file is not readable.
    #[error("key file {path} not readable: {message}")]
    Unreadable {
        /// Path of the file.
        path: PathBuf,
        /// Error message from the operating system.
        message: String,
    },
}

/// 256-bit key; overwritten in memory when dropped.
pub struct MasterKey(Zeroizing<[u8; KEY_BYTES]>);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<hidden>)")
    }
}

impl MasterKey {
    /// Reads the key in this order: systemd credential
    /// (`$CREDENTIALS_DIRECTORY/protonsync-key`), file from `PROTONSYNC_KEY_FILE`,
    /// value from `PROTONSYNC_KEY`.
    ///
    /// # Errors
    /// [`KeyError`] if nothing is set or the key is invalid.
    pub fn from_environment() -> Result<Self, KeyError> {
        if let Some(dir) = std::env::var_os("CREDENTIALS_DIRECTORY") {
            let path = Path::new(&dir).join(KEY_CREDENTIAL_NAME);
            if path.exists() {
                // systemd creates credentials with safe permissions itself.
                return Self::read(&path);
            }
        }
        if let Some(path) = std::env::var_os(KEY_FILE_ENV) {
            return Self::from_file(Path::new(&path));
        }
        if let Ok(value) = std::env::var(KEY_ENV) {
            return Self::from_hex(&Zeroizing::new(value));
        }
        Err(KeyError::Missing)
    }

    /// Reads the key from a file that only the owner may read.
    ///
    /// # Errors
    /// [`KeyError::TooOpen`] if permissions are too open, otherwise read or format errors.
    pub fn from_file(path: &Path) -> Result<Self, KeyError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(path).map_err(|e| unreadable(path, &e))?;
            if metadata.permissions().mode() & GROUP_AND_OTHER_BITS != 0 {
                return Err(KeyError::TooOpen(path.to_path_buf()));
            }
        }
        Self::read(path)
    }

    fn read(path: &Path) -> Result<Self, KeyError> {
        let text = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| unreadable(path, &e))?);
        Self::from_hex(&text)
    }

    /// Creates the key from 64 hex characters (surrounding whitespace is ignored).
    ///
    /// # Errors
    /// [`KeyError::Malformed`] on wrong length or invalid characters.
    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        let text = text.trim();
        if text.len() != KEY_BYTES * 2 {
            return Err(KeyError::Malformed);
        }
        let mut key = Zeroizing::new([0u8; KEY_BYTES]);
        for (index, byte) in key.iter_mut().enumerate() {
            let pair = text
                .get(index * 2..index * 2 + 2)
                .ok_or(KeyError::Malformed)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| KeyError::Malformed)?;
        }
        Ok(Self(key))
    }

    pub(crate) fn bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

fn unreadable(path: &Path, error: &std::io::Error) -> KeyError {
    KeyError::Unreadable {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn parses_hex_keys() {
        let key = MasterKey::from_hex(&format!("  {HEX}\n")).unwrap();
        assert_eq!(key.bytes()[31], 0x1f);
        assert!(matches!(
            MasterKey::from_hex("abc"),
            Err(KeyError::Malformed)
        ));
        let invalid = HEX.replace('0', "x");
        assert!(matches!(
            MasterKey::from_hex(&invalid),
            Err(KeyError::Malformed)
        ));
    }

    #[test]
    fn debug_hides_the_key() {
        let key = MasterKey::from_hex(HEX).unwrap();
        assert_eq!(format!("{key:?}"), "MasterKey(<hidden>)");
    }

    #[cfg(unix)]
    #[test]
    fn key_files_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, HEX).unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            MasterKey::from_file(&path),
            Err(KeyError::TooOpen(_))
        ));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(MasterKey::from_file(&path).is_ok());

        let missing = dir.path().join("missing");
        assert!(matches!(
            MasterKey::from_file(&missing),
            Err(KeyError::Unreadable { .. })
        ));
    }
}

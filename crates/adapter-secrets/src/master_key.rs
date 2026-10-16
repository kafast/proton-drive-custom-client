//! Der Schlüssel, mit dem der Server seine Geheimnisdatei verschlüsselt.

use std::path::{Path, PathBuf};

use thiserror::Error;
use zeroize::Zeroizing;

/// Name des systemd-Credentials (`LoadCredential=protonsync-key:…`).
pub const KEY_CREDENTIAL_NAME: &str = "protonsync-key";
/// Umgebungsvariable mit dem Pfad einer Schlüsseldatei.
pub const KEY_FILE_ENV: &str = "PROTONSYNC_KEY_FILE";
/// Umgebungsvariable mit dem Schlüssel selbst (z. B. als Container-Secret).
pub const KEY_ENV: &str = "PROTONSYNC_KEY";

/// Länge des Schlüssels in Bytes (AES-256).
const KEY_BYTES: usize = 32;
/// Rechte, die für Gruppe und andere gesetzt sein dürfen: keine.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;

/// Fehler beim Beschaffen des Schlüssels.
#[derive(Debug, Error)]
pub enum KeyError {
    /// Weder Credential noch Datei noch Variable ist gesetzt.
    #[error(
        "Kein Schlüssel gefunden: systemd-Credential {KEY_CREDENTIAL_NAME}, {KEY_FILE_ENV} oder {KEY_ENV} setzen"
    )]
    Missing,
    /// Die Datei ist für andere lesbar.
    #[error("Schlüsseldatei {0} ist für andere lesbar; bitte `chmod 600` ausführen")]
    TooOpen(PathBuf),
    /// Der Inhalt ist kein Schlüssel aus 64 Hex-Zeichen.
    #[error("Schlüssel muss aus 64 Hex-Zeichen bestehen (z. B. `openssl rand -hex 32`)")]
    Malformed,
    /// Die Datei ist nicht lesbar.
    #[error("Schlüsseldatei {path} nicht lesbar: {message}")]
    Unreadable {
        /// Pfad der Datei.
        path: PathBuf,
        /// Fehlermeldung des Betriebssystems.
        message: String,
    },
}

/// 256-Bit-Schlüssel; wird beim Verwerfen im Speicher überschrieben.
pub struct MasterKey(Zeroizing<[u8; KEY_BYTES]>);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<verborgen>)")
    }
}

impl MasterKey {
    /// Liest den Schlüssel in dieser Reihenfolge: systemd-Credential
    /// (`$CREDENTIALS_DIRECTORY/protonsync-key`), Datei aus `PROTONSYNC_KEY_FILE`,
    /// Wert aus `PROTONSYNC_KEY`.
    ///
    /// # Errors
    /// [`KeyError`], wenn nichts gesetzt oder der Schlüssel ungültig ist.
    pub fn from_environment() -> Result<Self, KeyError> {
        if let Some(dir) = std::env::var_os("CREDENTIALS_DIRECTORY") {
            let path = Path::new(&dir).join(KEY_CREDENTIAL_NAME);
            if path.exists() {
                // systemd legt Credentials selbst mit sicheren Rechten an.
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

    /// Liest den Schlüssel aus einer Datei, die nur der Eigentümer lesen darf.
    ///
    /// # Errors
    /// [`KeyError::TooOpen`] bei zu offenen Rechten, sonst Lese- oder Formatfehler.
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

    /// Erzeugt den Schlüssel aus 64 Hex-Zeichen (Leerraum am Rand wird ignoriert).
    ///
    /// # Errors
    /// [`KeyError::Malformed`] bei falscher Länge oder ungültigen Zeichen.
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
        assert_eq!(format!("{key:?}"), "MasterKey(<verborgen>)");
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

        let missing = dir.path().join("fehlt");
        assert!(matches!(
            MasterKey::from_file(&missing),
            Err(KeyError::Unreadable { .. })
        ));
    }
}

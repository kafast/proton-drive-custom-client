//! Encrypted secret files for the server.
//!
//! Format of each file `<name>.secret`: magic `PSS1`, 12-byte nonce, then the
//! AES-256-GCM encrypted content. The secret's name goes in as additional
//! authenticated data, so that a renamed file cannot pass as a different
//! secret.

use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};

use crate::{MasterKey, validate_key};

const MAGIC: &[u8; 4] = b"PSS1";
const NONCE_BYTES: usize = 12;
const FILE_SUFFIX: &str = "secret";
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Secrets as encrypted files in a directory.
pub struct EncryptedFileStore {
    dir: PathBuf,
    cipher: Aes256Gcm,
}

impl EncryptedFileStore {
    /// Opens (and creates if needed) the directory `dir` with permissions 0700.
    ///
    /// # Errors
    /// [`SecretError::Unavailable`] if the directory cannot be created.
    pub fn open(dir: impl Into<PathBuf>, key: &MasterKey) -> Result<Self, SecretError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
                .map_err(|e| io_error(&dir, &e))?;
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.bytes()));
        Ok(Self { dir, cipher })
    }

    fn path_of(&self, key: &SecretKey) -> Result<PathBuf, SecretError> {
        let name = validate_key(key)?;
        Ok(self.dir.join(format!("{name}.{FILE_SUFFIX}")))
    }

    fn encrypt(&self, name: &str, plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Unavailable("encryption failed".into()))?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_BYTES + ciphertext.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn decrypt(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, SecretError> {
        let corrupt = || {
            SecretError::Unavailable(format!(
                "secret {name} cannot be decrypted (wrong key or corrupted)"
            ))
        };
        let rest = data.strip_prefix(MAGIC.as_slice()).ok_or_else(corrupt)?;
        if rest.len() < NONCE_BYTES {
            return Err(corrupt());
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_BYTES);
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| corrupt())
    }
}

impl SecretStore for EncryptedFileStore {
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        let path = self.path_of(key)?;
        match std::fs::read(&path) {
            Ok(data) => self
                .decrypt(key.as_str(), &data)
                .map(|bytes| Some(Secret::new(bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(&path, &e)),
        }
    }

    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        let path = self.path_of(key)?;
        let data = self.encrypt(key.as_str(), secret.expose())?;
        // First write completely into a temporary file (permissions 0600), then
        // rename atomically: a crash never leaves a half-written file.
        let mut temp = tempfile::Builder::new()
            .prefix(".tmp-")
            .tempfile_in(&self.dir)
            .map_err(|e| io_error(&self.dir, &e))?;
        temp.write_all(&data)
            .map_err(|e| io_error(temp.path(), &e))?;
        temp.as_file()
            .sync_all()
            .map_err(|e| io_error(temp.path(), &e))?;
        temp.persist(&path).map_err(|e| io_error(&path, &e.error))?;
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let path = self.path_of(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error(&path, &e)),
        }
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> SecretError {
    SecretError::Unavailable(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const KEY_B: &str = "ff0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn store_in(dir: &Path, hex: &str) -> EncryptedFileStore {
        EncryptedFileStore::open(dir, &MasterKey::from_hex(hex).unwrap()).unwrap()
    }

    #[test]
    fn stores_loads_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path(), KEY_A);
        let key = SecretKey::new("session");
        assert_eq!(store.load(&key).unwrap(), None);

        store.store(&key, &Secret::new(b"secret".to_vec())).unwrap();
        assert_eq!(
            store.load(&key).unwrap(),
            Some(Secret::new(b"secret".to_vec()))
        );

        store.store(&key, &Secret::new(b"new".to_vec())).unwrap();
        assert_eq!(
            store.load(&key).unwrap(),
            Some(Secret::new(b"new".to_vec()))
        );

        store.delete(&key).unwrap();
        store.delete(&key).unwrap();
        assert_eq!(store.load(&key).unwrap(), None);
    }

    #[test]
    fn file_does_not_contain_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path(), KEY_A);
        store
            .store(
                &SecretKey::new("session"),
                &Secret::new(b"plaintext-token".to_vec()),
            )
            .unwrap();
        let data = std::fs::read(dir.path().join("session.secret")).unwrap();
        assert!(!data.windows(9).any(|w| w == b"plaintext"));
    }

    #[test]
    fn wrong_key_or_renamed_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        store_in(dir.path(), KEY_A)
            .store(&SecretKey::new("a"), &Secret::new(b"x".to_vec()))
            .unwrap();
        assert!(
            store_in(dir.path(), KEY_B)
                .load(&SecretKey::new("a"))
                .is_err()
        );

        std::fs::rename(dir.path().join("a.secret"), dir.path().join("b.secret")).unwrap();
        assert!(
            store_in(dir.path(), KEY_A)
                .load(&SecretKey::new("b"))
                .is_err()
        );

        std::fs::write(dir.path().join("c.secret"), b"PSS1short").unwrap();
        assert!(
            store_in(dir.path(), KEY_A)
                .load(&SecretKey::new("c"))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn files_and_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join("secrets");
        let store = store_in(&secrets, KEY_A);
        store
            .store(&SecretKey::new("a"), &Secret::new(b"x".to_vec()))
            .unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&secrets), 0o700);
        assert_eq!(mode(&secrets.join("a.secret")), 0o600);
    }
}

//! Anmeldung an der Web-Oberfläche: Passwort-Hash, Sitzungen, CSRF-Schutz und
//! Bremse gegen Passwort-Raten.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use tokio::time::Instant;

/// Länge zufälliger Kennungen (Sitzung, CSRF) in Bytes.
const TOKEN_BYTES: usize = 32;
/// Nach so vielen Fehlversuchen von einer Adresse wird gesperrt.
const MAX_FAILURES: u32 = 5;
/// So lange bleibt eine Adresse nach zu vielen Fehlversuchen gesperrt.
const LOCKOUT: Duration = Duration::from_mins(5);

/// Fehler rund um Passwörter.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Der gespeicherte Hash ist kein gültiger Argon2-Hash.
    #[error(
        "password_hash ist kein gültiger Argon2-Hash (mit `protonsyncd hash-password` erzeugen)"
    )]
    InvalidHash,
    /// Zufallszahlen nicht verfügbar.
    #[error("Zufallszahlen nicht verfügbar: {0}")]
    Random(String),
}

/// Erzeugt einen Argon2id-Hash im PHC-Format (`$argon2id$…`) für die Konfiguration.
///
/// # Errors
/// [`AuthError::Random`], wenn das System keine Zufallszahlen liefert.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| AuthError::Random(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| AuthError::Random(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AuthError::Random(e.to_string()))
}

/// Der geprüfte Passwort-Hash aus der Konfiguration.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordCheck(String);

impl PasswordCheck {
    /// Prüft das Format des Hashes.
    ///
    /// # Errors
    /// [`AuthError::InvalidHash`] bei ungültigem Format.
    pub fn new(hash: &str) -> Result<Self, AuthError> {
        PasswordHash::new(hash).map_err(|_| AuthError::InvalidHash)?;
        Ok(Self(hash.to_owned()))
    }

    /// `true`, wenn `password` passt.
    #[must_use]
    pub fn verify(&self, password: &str) -> bool {
        PasswordHash::new(&self.0).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
    }
}

impl std::fmt::Debug for PasswordCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordCheck(<verborgen>)")
    }
}

/// Eine zufällige Kennung als Hex-Text.
///
/// # Errors
/// [`AuthError::Random`], wenn das System keine Zufallszahlen liefert.
pub fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|e| AuthError::Random(e.to_string()))?;
    let mut text = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// Vergleicht zwei Texte in gleichbleibender Zeit (gegen Zeitmessungs-Angriffe).
#[must_use]
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

struct Session {
    csrf: String,
    expires: Instant,
}

/// Angemeldete Sitzungen im Speicher (nach einem Neustart neu anmelden).
pub struct Sessions {
    lifetime: Duration,
    sessions: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// Sitzungen, die nach `lifetime` ablaufen.
    #[must_use]
    pub fn new(lifetime: Duration) -> Self {
        Self {
            lifetime,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Wie lange eine Sitzung gilt.
    #[must_use]
    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// Legt eine Sitzung an und liefert ihre Kennung.
    ///
    /// # Errors
    /// [`AuthError::Random`] ohne Zufallszahlen.
    pub fn create(&self) -> Result<String, AuthError> {
        let token = random_token()?;
        let session = Session {
            csrf: random_token()?,
            expires: Instant::now() + self.lifetime,
        };
        if let Ok(mut sessions) = self.sessions.lock() {
            let now = Instant::now();
            sessions.retain(|_, s| s.expires > now);
            sessions.insert(token.clone(), session);
        }
        Ok(token)
    }

    /// CSRF-Kennung einer gültigen Sitzung.
    #[must_use]
    pub fn csrf(&self, token: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        sessions
            .get(token)
            .filter(|session| session.expires > Instant::now())
            .map(|session| session.csrf.clone())
    }

    /// Beendet eine Sitzung.
    pub fn remove(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }
}

/// Bremse gegen Passwort-Raten: nach [`MAX_FAILURES`] Fehlversuchen bleibt
/// die Adresse [`LOCKOUT`] lang gesperrt.
#[derive(Default)]
pub struct LoginGuard {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl LoginGuard {
    /// `true`, wenn von `ip` gerade ein Versuch erlaubt ist.
    #[must_use]
    pub fn allowed(&self, ip: IpAddr) -> bool {
        let Ok(failures) = self.failures.lock() else {
            return false;
        };
        failures
            .get(&ip)
            .is_none_or(|(count, last)| *count < MAX_FAILURES || last.elapsed() >= LOCKOUT)
    }

    /// Merkt einen Fehlversuch.
    pub fn failed(&self, ip: IpAddr) {
        if let Ok(mut failures) = self.failures.lock() {
            let entry = failures.entry(ip).or_insert((0, Instant::now()));
            if entry.1.elapsed() >= LOCKOUT {
                entry.0 = 0;
            }
            entry.0 += 1;
            entry.1 = Instant::now();
        }
    }

    /// Erfolgreiche Anmeldung: Zähler zurücksetzen.
    pub fn succeeded(&self, ip: IpAddr) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(&ip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_only_the_right_password() {
        let hash = hash_password("geheim").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        let check = PasswordCheck::new(&hash).unwrap();
        assert!(check.verify("geheim"));
        assert!(!check.verify("Geheim"));
        assert_eq!(format!("{check:?}"), "PasswordCheck(<verborgen>)");
        assert!(PasswordCheck::new("klartext").is_err());
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token().unwrap(), random_token().unwrap());
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(constant_time_eq(&a, &a.clone()));
        assert!(!constant_time_eq(&a, &b));
        assert!(!constant_time_eq("a", "ab"));
    }

    #[tokio::test(start_paused = true)]
    async fn sessions_expire() {
        let sessions = Sessions::new(Duration::from_hours(1));
        let token = sessions.create().unwrap();
        assert!(sessions.csrf(&token).is_some());
        assert!(sessions.csrf("fremd").is_none());
        tokio::time::advance(Duration::from_mins(61)).await;
        assert!(sessions.csrf(&token).is_none());
        let token = sessions.create().unwrap();
        sessions.remove(&token);
        assert!(sessions.csrf(&token).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn too_many_failures_lock_the_address_for_a_while() {
        let guard = LoginGuard::default();
        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        let other: IpAddr = "192.168.1.10".parse().unwrap();
        for _ in 0..MAX_FAILURES {
            assert!(guard.allowed(ip));
            guard.failed(ip);
        }
        assert!(!guard.allowed(ip));
        assert!(guard.allowed(other));
        tokio::time::advance(LOCKOUT).await;
        assert!(guard.allowed(ip));
        guard.failed(ip);
        assert!(guard.allowed(ip), "Zähler beginnt nach der Sperre neu");
        guard.succeeded(ip);
        assert!(guard.allowed(ip));
    }
}

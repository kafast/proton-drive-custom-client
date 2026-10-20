//! Login to the web interface: password hash, sessions, CSRF protection and
//! a brake against password guessing.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use tokio::time::Instant;

/// Length of random tokens (session, CSRF) in bytes.
const TOKEN_BYTES: usize = 32;
/// After this many failed attempts from one address, it is locked.
const MAX_FAILURES: u32 = 5;
/// How long an address stays locked after too many failed attempts.
const LOCKOUT: Duration = Duration::from_mins(5);

/// Errors around passwords.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The stored hash is not a valid Argon2 hash.
    #[error(
        "password_hash is not a valid Argon2 hash (create it with `protonsyncd hash-password`)"
    )]
    InvalidHash,
    /// Random numbers not available.
    #[error("random numbers not available: {0}")]
    Random(String),
}

/// Creates an Argon2id hash in PHC format (`$argon2id$…`) for the configuration.
///
/// # Errors
/// [`AuthError::Random`] if the system provides no random numbers.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    // `hash_password` draws a fresh random salt from the operating system.
    let hash: PasswordHash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| AuthError::Random(e.to_string()))?;
    Ok(hash.to_string())
}

/// The validated password hash from the configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordCheck(String);

impl PasswordCheck {
    /// Checks the format of the hash.
    ///
    /// # Errors
    /// [`AuthError::InvalidHash`] if the format is invalid.
    pub fn new(hash: &str) -> Result<Self, AuthError> {
        PasswordHash::new(hash).map_err(|_| AuthError::InvalidHash)?;
        Ok(Self(hash.to_owned()))
    }

    /// `true` if `password` matches.
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
        f.write_str("PasswordCheck(<hidden>)")
    }
}

/// A random token as hex text.
///
/// # Errors
/// [`AuthError::Random`] if the system provides no random numbers.
pub fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| AuthError::Random(e.to_string()))?;
    let mut text = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// Compares two texts in constant time (against timing attacks).
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

/// Logged-in sessions in memory (log in again after a restart).
pub struct Sessions {
    lifetime: Duration,
    sessions: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// Sessions that expire after `lifetime`.
    #[must_use]
    pub fn new(lifetime: Duration) -> Self {
        Self {
            lifetime,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// How long a session stays valid.
    #[must_use]
    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// Creates a session and returns its token.
    ///
    /// # Errors
    /// [`AuthError::Random`] without random numbers.
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

    /// CSRF token of a valid session.
    #[must_use]
    pub fn csrf(&self, token: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        sessions
            .get(token)
            .filter(|session| session.expires > Instant::now())
            .map(|session| session.csrf.clone())
    }

    /// Ends a session.
    pub fn remove(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }
}

/// Brake against password guessing: after [`MAX_FAILURES`] failed attempts the
/// address stays locked for [`LOCKOUT`].
#[derive(Default)]
pub struct LoginGuard {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl LoginGuard {
    /// `true` if an attempt from `ip` is currently allowed.
    #[must_use]
    pub fn allowed(&self, ip: IpAddr) -> bool {
        let Ok(failures) = self.failures.lock() else {
            return false;
        };
        failures
            .get(&ip)
            .is_none_or(|(count, last)| *count < MAX_FAILURES || last.elapsed() >= LOCKOUT)
    }

    /// Records a failed attempt.
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

    /// Successful login: reset the counter.
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
        let hash = hash_password("secret").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        let check = PasswordCheck::new(&hash).unwrap();
        assert!(check.verify("secret"));
        assert!(!check.verify("Secret"));
        assert_eq!(format!("{check:?}"), "PasswordCheck(<hidden>)");
        assert!(PasswordCheck::new("plaintext").is_err());
    }

    /// Hashes in existing configurations (created with argon2 0.5) keep working.
    #[test]
    fn verifies_hashes_from_earlier_versions() {
        let check = PasswordCheck::new(
            "$argon2id$v=19$m=19456,t=2,p=1$zoFo2aPmLzXkBPD5OB2HZw$IbEDtA1meQjc7tRQXymXJi07K3pN5WCmh2tcjV/EDZg",
        )
        .unwrap();
        assert!(check.verify("saved-before-0.6"));
        assert!(!check.verify("wrong"));
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
        assert!(sessions.csrf("unknown").is_none());
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
        assert!(guard.allowed(ip), "counter starts over after the lockout");
        guard.succeeded(ip);
        assert!(guard.allowed(ip));
    }
}

//! Übertragung: eine JSON-Nachricht pro Zeile, und wo der Socket liegt.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Umgebungsvariable, die den Socket-Pfad überschreibt.
pub const SOCKET_ENV: &str = "PROTONSYNC_SOCKET";
/// Socket-Pfad, wenn weder [`SOCKET_ENV`] noch `XDG_RUNTIME_DIR` gesetzt ist
/// (Server-Betrieb mit systemd `RuntimeDirectory=protonsync`).
pub const SYSTEM_SOCKET: &str = "/run/protonsync/daemon.sock";
/// Längste erlaubte Zeile; schützt den Daemon vor übergroßen Anfragen.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Fehler beim Kodieren oder Dekodieren.
#[derive(Debug, Error)]
pub enum WireError {
    /// Kein gültiges JSON für diesen Nachrichtentyp.
    #[error("Ungültige Nachricht: {0}")]
    Invalid(String),
    /// Zeile länger als [`MAX_LINE_BYTES`].
    #[error("Nachricht zu groß")]
    TooLarge,
}

/// Kodiert eine Nachricht als eine Zeile (mit abschließendem `\n`).
///
/// # Errors
/// [`WireError::Invalid`], wenn sich der Wert nicht als JSON darstellen lässt.
pub fn encode<T: Serialize>(message: &T) -> Result<String, WireError> {
    let mut line = serde_json::to_string(message).map_err(|e| WireError::Invalid(e.to_string()))?;
    line.push('\n');
    Ok(line)
}

/// Dekodiert eine Zeile (mit oder ohne `\n`).
///
/// # Errors
/// [`WireError`] bei zu langer oder ungültiger Zeile.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, WireError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(WireError::TooLarge);
    }
    serde_json::from_str(line.trim_end()).map_err(|e| WireError::Invalid(e.to_string()))
}

/// Socket-Pfad: [`SOCKET_ENV`], sonst `$XDG_RUNTIME_DIR/protonsync/daemon.sock`,
/// sonst [`SYSTEM_SOCKET`].
#[must_use]
pub fn default_socket_path() -> PathBuf {
    socket_path_from(
        std::env::var_os(SOCKET_ENV).map(PathBuf::from),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
    )
}

fn socket_path_from(explicit: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| runtime_dir.map(|dir| dir.join("protonsync").join("daemon.sock")))
        .unwrap_or_else(|| PathBuf::from(SYSTEM_SOCKET))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, Request};

    #[test]
    fn messages_are_single_lines() {
        let line = encode(&ClientMessage::new(Request::Status)).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: ClientMessage = decode(&line).unwrap();
        assert_eq!(back.request, Request::Status);
    }

    #[test]
    fn garbage_and_oversized_lines_are_rejected() {
        assert!(matches!(
            decode::<ClientMessage>("{"),
            Err(WireError::Invalid(_))
        ));
        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        assert!(matches!(
            decode::<ClientMessage>(&huge),
            Err(WireError::TooLarge)
        ));
    }

    #[test]
    fn socket_path_prefers_explicit_then_runtime_dir() {
        let explicit = Some(PathBuf::from("/tmp/x.sock"));
        let runtime = Some(PathBuf::from("/run/user/1000"));
        assert_eq!(
            socket_path_from(explicit, runtime.clone()),
            PathBuf::from("/tmp/x.sock")
        );
        assert_eq!(
            socket_path_from(None, runtime),
            PathBuf::from("/run/user/1000/protonsync/daemon.sock")
        );
        assert_eq!(socket_path_from(None, None), PathBuf::from(SYSTEM_SOCKET));
    }
}

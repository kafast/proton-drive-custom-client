//! Fehler der Domäne.

use thiserror::Error;

/// Verletzung einer Domänenregel, z. B. ein ungültiger Dateiname.
///
/// `#[derive(Error)]` stammt aus `thiserror` und erzeugt die `Display`-Ausgabe
/// aus den `#[error(...)]`-Texten, ähnlich einer Exception-Message in Java.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// Ein Knotenname verletzt die Namensregeln.
    #[error("ungültiger Name {name:?}: {reason}")]
    InvalidNodeName {
        /// Der abgelehnte Name.
        name: String,
        /// Warum der Name abgelehnt wurde.
        reason: &'static str,
    },
    /// Eine Aufbewahrungs- oder Freigabefrist von null Tagen ist nicht sinnvoll.
    #[error("die Frist muss mindestens einen Tag betragen")]
    ZeroDays,
    /// Ein Cron-Ausdruck ist ungültig.
    #[error("ungültiger Zeitplan {expression:?}: {reason}")]
    InvalidCron {
        /// Der abgelehnte Ausdruck.
        expression: String,
        /// Warum er abgelehnt wurde.
        reason: String,
    },
    /// Ein Dateimuster ist ungültig.
    #[error("ungültiges Dateimuster {pattern:?}: {reason}")]
    InvalidGlob {
        /// Das abgelehnte Muster.
        pattern: String,
        /// Warum es abgelehnt wurde.
        reason: &'static str,
    },
    /// Eine Kennung (Sync-Paar, Upload-Job) verletzt die Namensregeln.
    #[error("ungültige Kennung {0:?} (erlaubt: a-z, 0-9, -, _)")]
    InvalidId(String),
    /// Eine Aufbewahrungsregel ist widersprüchlich oder leer.
    #[error("ungültige Aufbewahrung: {0}")]
    InvalidRetention(&'static str),
}

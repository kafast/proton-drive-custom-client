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
}

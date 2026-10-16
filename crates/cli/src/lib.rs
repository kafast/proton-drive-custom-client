//! Kommandozeile `protonsync`. Enthält keine Geschäftslogik: Jeder Befehl wird
//! als Anfrage an den Daemon geschickt (Daemon-API) und die Antwort ausgegeben.

pub mod commands;
pub mod render;

// Verbindung und Sprache liegen im gemeinsamen Crate `client`; die alten
// Pfade `protonsync_cli::client` und `protonsync_cli::i18n` bleiben gültig.
pub use protonsync_client::{i18n, transport as client};

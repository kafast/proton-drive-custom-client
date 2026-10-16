//! Kommandozeile `protonsync`. Enthält keine Geschäftslogik: Jeder Befehl wird
//! als Anfrage an den Daemon geschickt (Daemon-API) und die Antwort ausgegeben.

pub mod client;
pub mod commands;
pub mod i18n;
pub mod render;

//! Daemon von protonsync: der einzige Ort mit Geschäftsablauf.
//!
//! - [`config`] – Konfigurationsdatei.
//! - [`core`] – beantwortet API-Anfragen und führt Abgleiche aus (generisch über die Ports).
//! - [`scheduler`] – Zeitplan (Ereignisse, lokale Änderungen, Sicherheitsnetz, Papierkorb).
//! - [`server`] – Unix-Socket-Server der Daemon-API.

pub mod config;
mod convert;
pub mod core;

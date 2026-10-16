//! Daemon von protonsync: der einzige Ort mit Geschäftsablauf.
//!
//! - [`app`] – Zusammenbau der echten Adapter (Composition Root).
//! - [`config`] – Konfigurationsdatei.
//! - [`core`] – beantwortet API-Anfragen und führt Abgleiche aus (generisch über die Ports).
//! - [`scheduler`] – Zeitplan (Ereignisse, lokale Änderungen, Sicherheitsnetz, Papierkorb).
//! - [`server`] – Unix-Socket-Server der Daemon-API.
//! - `fuse_mount` – Einhängen der Laufwerke über FUSE (Feature `fuse`, nur Linux).

pub mod app;
pub mod config;
mod convert;
pub mod core;
#[cfg(all(feature = "fuse", target_os = "linux"))]
pub mod fuse_mount;
pub mod job_config;
pub mod local_time;
pub mod mount_config;
pub mod scheduler;
pub mod server;
pub mod web;
pub mod web_config;

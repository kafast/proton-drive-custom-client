//! The protonsync daemon: the only place with business workflow.
//!
//! - [`app`] – wiring of the real adapters (composition root).
//! - [`config`] – configuration file.
//! - [`config_example`] – the commented example configuration with all defaults.
//! - [`core`] – answers API requests and runs syncs (generic over the ports).
//! - [`scheduler`] – schedule (events, local changes, safety net, trash).
//! - [`server`] – Unix socket server of the daemon API.
//! - [`notify`] – notifications by email/webhook and the weekly report.
//! - `fuse_mount` – mounting the drives via FUSE (feature `fuse`, Linux only).

pub mod app;
#[cfg(feature = "keyring")]
mod blocking_secrets;
pub mod config;
pub mod config_example;
mod convert;
pub mod core;
#[cfg(all(feature = "fuse", target_os = "linux"))]
pub mod fuse_mount;
pub mod job_config;
pub mod local_time;
pub mod mount_config;
pub mod notify;
pub mod notify_config;
pub mod photo_config;
pub mod scheduler;
pub mod server;
pub mod web;
pub mod web_config;

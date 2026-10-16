//! FUSE-Dateisystem für das virtuelle Laufwerk (Platzhalter-Modus, ADR 0010).
//!
//! Das Crate übersetzt Anfragen des Linux-Kernels in Aufrufe von
//! [`VirtualDrive`](protonsync_application::drive::VirtualDrive). Es enthält
//! keine eigene Logik:
//!
//! - [`ops`]: die Übersetzung selbst (Namen, Attribute, Fehlercodes), ohne
//!   Kernel testbar.
//! - [`mount`]: Einhängen über `fuser` ohne libfuse; nötig ist zur Laufzeit nur
//!   `fusermount3` (Paket `fuse3`).
//!
//! Jede Kernel-Anfrage läuft als eigene tokio-Aufgabe. So blockiert ein
//! langsamer Download nicht das ganze Laufwerk.
//!
//! Nicht unterstützt: harte und symbolische Links, erweiterte Attribute;
//! Rechte (`chmod`) und Besitzer werden angenommen, aber nicht gespeichert.

// FUSE gibt es in dieser Form nur unter Linux (unter Windows später die
// Cloud Files API, siehe CLAUDE.md Abschnitt 9).
#![cfg(target_os = "linux")]

mod errno;
mod filesystem;
pub mod mount;
pub mod ops;

pub use errno::errno_of;
pub use mount::{MountError, MountHandle, mount};
pub use ops::{DriveOps, Owner};

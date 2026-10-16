//! Domäne von protonsync (innerste Schicht der Clean Architecture).
//!
//! Dieses Crate kennt weder Proton noch das Dateisystem noch eine Datenbank.
//! Es enthält nur Datentypen und reine Regeln, die sich ohne I/O testen lassen.

pub mod cache;
pub mod conflict;
pub mod error;
pub mod job;
pub mod node;
pub mod path;
pub mod photo;
pub mod pin;
pub mod sync_pair;
pub mod time;
pub mod trash;

pub use error::DomainError;

//! Domain of protonsync (innermost layer of the Clean Architecture).
//!
//! This crate knows neither Proton nor the file system nor a database.
//! It contains only data types and pure rules that can be tested without I/O.

pub mod archive;
pub mod cache;
pub mod change_log;
pub mod conflict;
pub mod disk;
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

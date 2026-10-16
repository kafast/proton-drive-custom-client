//! Contract tests: behavior that every implementation of a port must show.
//!
//! Each port has a macro that generates one test function per contract test.
//! This way the same tests run against fakes and real adapters.

// This is test code: a failed `unwrap` is supposed to abort the test.
#![allow(clippy::unwrap_used, clippy::missing_panics_doc)]

pub mod drive;
pub mod job_log;
pub mod local;
pub mod photos;
pub mod remote;
pub mod state;

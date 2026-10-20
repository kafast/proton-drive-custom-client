//! FUSE file system for the virtual drive (placeholder mode, ADR 0010).
//!
//! The crate translates requests from the Linux kernel into calls to
//! [`VirtualDrive`](protonsync_application::drive::VirtualDrive). It contains
//! no logic of its own:
//!
//! - [`ops`]: the translation itself (names, attributes, error codes), testable
//!   without a kernel.
//! - [`mount`](mod@mount): mounting via `fuser` without libfuse; at runtime only
//!   `fusermount3` (package `fuse3`) is needed.
//!
//! Every kernel request runs as its own tokio task, so a slow download does
//! not block the whole drive.
//!
//! Not supported: hard and symbolic links, extended attributes; permissions
//! (`chmod`) and owners are accepted but not stored.

// FUSE in this form only exists on Linux (on Windows later the Cloud Files
// API, see CLAUDE.md section 9).
#![cfg(target_os = "linux")]

mod errno;
mod filesystem;
pub mod mount;
pub mod ops;

pub use errno::errno_of;
pub use mount::{MountError, MountHandle, mount};
pub use ops::{DriveOps, Owner};

//! Application layer of protonsync.
//!
//! This is where the **ports** live: traits (comparable to Java interfaces) through
//! which the use cases talk to the outside world. They are implemented in the
//! adapter crates (e.g. `adapter-proton`) and, for tests, in the `testkit`.
//! This crate depends only on `protonsync-domain`.

pub mod drive;
pub mod photos;
pub mod ports;
pub mod sync;
pub mod use_cases;

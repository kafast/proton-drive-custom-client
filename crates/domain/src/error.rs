//! Domain errors.

use thiserror::Error;

/// Violation of a domain rule, e.g. an invalid file name.
///
/// `#[derive(Error)]` comes from `thiserror` and generates the `Display` output
/// from the `#[error(...)]` texts, similar to an exception message in Java.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// A node name violates the naming rules.
    #[error("invalid name {name:?}: {reason}")]
    InvalidNodeName {
        /// The rejected name.
        name: String,
        /// Why the name was rejected.
        reason: &'static str,
    },
    /// A retention or eviction period of zero days makes no sense.
    #[error("the period must be at least one day")]
    ZeroDays,
    /// A cron expression is invalid.
    #[error("invalid schedule {expression:?}: {reason}")]
    InvalidCron {
        /// The rejected expression.
        expression: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A file pattern is invalid.
    #[error("invalid file pattern {pattern:?}: {reason}")]
    InvalidGlob {
        /// The rejected pattern.
        pattern: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// An id (sync pair, upload job) violates the naming rules.
    #[error("invalid id {0:?} (allowed: a-z, 0-9, -, _)")]
    InvalidId(String),
    /// A retention rule is contradictory or empty.
    #[error("invalid retention: {0}")]
    InvalidRetention(&'static str),
}

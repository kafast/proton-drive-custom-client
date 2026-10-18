//! Command line `protonsync`. Contains no business logic: every command is sent
//! to the daemon as a request (daemon API) and the response is printed.

pub mod commands;
pub mod render;

// Connection and language live in the shared crate `client`; the old paths
// `protonsync_cli::client` and `protonsync_cli::i18n` remain valid.
pub use protonsync_client::{i18n, transport as client};

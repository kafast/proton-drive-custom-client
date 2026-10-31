//! Shared building blocks for programs that talk to the daemon (CLI, tray
//! app): connection over the Unix socket, event subscription and the output
//! language.
//!
//! The connection needs Unix sockets and exists only on Unix. The texts
//! (`changes`, `i18n`) stay portable, because the web interface uses them too
//! and is built on Windows as well (ADR 0018).

pub mod changes;
#[cfg(unix)]
pub mod events;
pub mod i18n;
#[cfg(unix)]
pub mod transport;

#[cfg(unix)]
pub use events::EventSubscription;
pub use i18n::Lang;
#[cfg(unix)]
pub use transport::{ClientError, SocketTransport, Transport};

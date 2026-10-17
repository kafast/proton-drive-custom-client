//! Shared building blocks for programs that talk to the daemon (CLI, tray
//! app): connection over the Unix socket, event subscription and the output
//! language.

pub mod events;
pub mod i18n;
pub mod transport;

pub use events::EventSubscription;
pub use i18n::Lang;
pub use transport::{ClientError, SocketTransport, Transport};

//! Gemeinsame Bausteine für Programme, die mit dem Daemon sprechen (CLI,
//! Tray-App): Verbindung über den Unix-Socket, Ereignis-Abonnement und die
//! Sprache der Ausgabe.

pub mod events;
pub mod i18n;
pub mod transport;

pub use events::EventSubscription;
pub use i18n::Lang;
pub use transport::{ClientError, SocketTransport, Transport};

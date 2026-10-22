//! Port for notifications to the user outside the desktop (e.g. email, webhook).
//!
//! What is reported and when is decided by the daemon; a notifier only delivers
//! a finished message.

use std::future::Future;

use thiserror::Error;

/// A finished message, already in the language of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Short subject (email subject, title of a push message).
    pub subject: String,
    /// Plain text body.
    pub body: String,
}

/// Why a message could not be delivered.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotifyError {
    /// The service is unreachable right now; a later attempt may work.
    #[error("notification service unavailable: {0}")]
    Unavailable(String),
    /// The service refused the message (wrong password, invalid address, …);
    /// trying again does not help.
    #[error("notification rejected: {0}")]
    Rejected(String),
}

impl NotifyError {
    /// May a later attempt succeed?
    #[must_use]
    pub const fn is_temporary(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// Delivers messages through one channel.
pub trait Notifier: Send + Sync {
    /// Short name of the channel for logs (e.g. `mail`).
    fn channel(&self) -> &str;

    /// Delivers `message`.
    ///
    /// # Errors
    /// [`NotifyError`] if the message did not arrive.
    fn send(&self, message: &Message) -> impl Future<Output = Result<(), NotifyError>> + Send;
}

/// A shared notifier (`Arc`) is itself a notifier.
impl<T: Notifier + ?Sized> Notifier for std::sync::Arc<T> {
    fn channel(&self) -> &str {
        (**self).channel()
    }

    fn send(&self, message: &Message) -> impl Future<Output = Result<(), NotifyError>> + Send {
        (**self).send(message)
    }
}

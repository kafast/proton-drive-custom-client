//! Notifications to the user outside the desktop (ADR 0017): email via SMTP and
//! webhooks (e.g. ntfy, Gotify, Home Assistant). Both implement the
//! [`Notifier`] port; what is reported and when is decided by the daemon.

pub mod mail;
pub mod webhook;

pub use mail::{MailNotifier, MailSecurity, MailSettings};
pub use webhook::{WebhookFormat, WebhookNotifier, WebhookSettings};

use protonsync_application::ports::{Message, Notifier, NotifyError};

/// One configured channel. An enum instead of `dyn Notifier`, because the
/// port uses `async fn` (ADR 0003) and is therefore not object safe.
pub enum Channel {
    /// Email.
    Mail(MailNotifier),
    /// Webhook.
    Webhook(WebhookNotifier),
}

impl Notifier for Channel {
    fn channel(&self) -> &str {
        match self {
            Self::Mail(mail) => mail.channel(),
            Self::Webhook(webhook) => webhook.channel(),
        }
    }

    async fn send(&self, message: &Message) -> Result<(), NotifyError> {
        match self {
            Self::Mail(mail) => mail.send(message).await,
            Self::Webhook(webhook) => webhook.send(message).await,
        }
    }
}

//! Email over SMTP (`lettre`, TLS with rustls).

use std::time::Duration;

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message as Email, Tokio1Executor};
use protonsync_application::ports::{Message, Notifier, NotifyError};

/// Waiting time for the mail server before an attempt counts as failed.
const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// How the connection to the mail server is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailSecurity {
    /// Plain connection upgraded with STARTTLS (usually port 587). Refuses servers without TLS.
    StartTls,
    /// TLS from the start (usually port 465).
    Tls,
    /// No encryption; only for a mail server on the same machine or in a trusted network.
    None,
}

/// Settings of the email channel.
#[derive(Clone, PartialEq, Eq)]
pub struct MailSettings {
    /// Host name of the SMTP server.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Encryption.
    pub security: MailSecurity,
    /// User name and password, if the server wants a login.
    pub login: Option<(String, String)>,
    /// Sender, e.g. `protonsync <me@example.org>`.
    pub from: String,
    /// Recipients.
    pub to: Vec<String>,
}

// Hand-written so that the password never ends up in a log.
impl std::fmt::Debug for MailSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("user", &self.login.as_ref().map(|(user, _)| user))
            .field("from", &self.from)
            .field("to", &self.to)
            .finish()
    }
}

/// Sends messages as plain text emails.
pub struct MailNotifier {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Vec<Mailbox>,
}

impl MailNotifier {
    /// Checks the addresses and prepares the connection (nothing is sent yet).
    ///
    /// # Errors
    /// [`NotifyError::Rejected`] for invalid addresses, no recipient or an invalid host name.
    pub fn new(settings: &MailSettings) -> Result<Self, NotifyError> {
        let from = parse_address(&settings.from)?;
        let to = settings
            .to
            .iter()
            .map(|address| parse_address(address))
            .collect::<Result<Vec<_>, _>>()?;
        if to.is_empty() {
            return Err(NotifyError::Rejected("no recipient".into()));
        }
        let builder = match settings.security {
            MailSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
                    .map_err(|e| NotifyError::Rejected(e.to_string()))?
            }
            MailSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
                .map_err(|e| NotifyError::Rejected(e.to_string()))?,
            MailSecurity::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.host)
            }
        };
        let mut builder = builder.port(settings.port).timeout(Some(SMTP_TIMEOUT));
        if let Some((user, password)) = &settings.login {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from,
            to,
        })
    }

    fn email(&self, message: &Message) -> Result<Email, NotifyError> {
        let mut builder = Email::builder()
            .from(self.from.clone())
            .subject(message.subject.clone())
            .header(ContentType::TEXT_PLAIN);
        for recipient in &self.to {
            builder = builder.to(recipient.clone());
        }
        builder
            .body(message.body.clone())
            .map_err(|e| NotifyError::Rejected(e.to_string()))
    }
}

fn parse_address(address: &str) -> Result<Mailbox, NotifyError> {
    address
        .parse()
        .map_err(|e| NotifyError::Rejected(format!("invalid email address {address:?}: {e}")))
}

impl Notifier for MailNotifier {
    fn channel(&self) -> &'static str {
        "mail"
    }

    async fn send(&self, message: &Message) -> Result<(), NotifyError> {
        let email = self.email(message)?;
        match self.transport.send(email).await {
            Ok(_) => Ok(()),
            Err(error) if error.is_permanent() => Err(NotifyError::Rejected(error.to_string())),
            Err(error) => Err(NotifyError::Unavailable(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> MailSettings {
        MailSettings {
            host: "localhost".into(),
            port: 25,
            security: MailSecurity::None,
            login: Some(("me".into(), "secret".into())),
            from: "protonsync <me@example.org>".into(),
            to: vec!["me@example.org".into()],
        }
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let mut bad_from = settings();
        bad_from.from = "not an address".into();
        assert!(matches!(
            MailNotifier::new(&bad_from),
            Err(NotifyError::Rejected(_))
        ));
        let mut nobody = settings();
        nobody.to.clear();
        assert!(MailNotifier::new(&nobody).is_err());
        assert!(MailNotifier::new(&settings()).is_ok());
    }

    #[test]
    fn debug_output_hides_the_password() {
        let text = format!("{:?}", settings());
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("me"));
    }
}

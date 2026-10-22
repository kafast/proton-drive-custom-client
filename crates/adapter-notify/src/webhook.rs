//! Webhooks: an HTTP POST per message (e.g. ntfy, Gotify, Home Assistant, own scripts).

use std::time::Duration;

use protonsync_application::ports::{Message, Notifier, NotifyError};
use reqwest::StatusCode;

/// Waiting time for the receiver.
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(15);
/// Sender name in the JSON body.
const SOURCE: &str = "protonsync";

/// Shape of the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookFormat {
    /// JSON `{"source": "protonsync", "title": …, "message": …}`.
    Json,
    /// The text as body and the subject in the `Title` header (understood by ntfy).
    Text,
}

/// Settings of the webhook channel.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookSettings {
    /// Target address (`http://` or `https://`).
    pub url: String,
    /// Body format.
    pub format: WebhookFormat,
    /// Sent as `Authorization: Bearer …`, if set.
    pub token: Option<String>,
}

// Hand-written so that the token never ends up in a log.
impl std::fmt::Debug for WebhookSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookSettings")
            .field("url", &self.url)
            .field("format", &self.format)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Posts messages to a URL.
pub struct WebhookNotifier {
    client: reqwest::Client,
    settings: WebhookSettings,
}

impl WebhookNotifier {
    /// Checks the URL and prepares the HTTP client.
    ///
    /// # Errors
    /// [`NotifyError::Rejected`] if the URL is not `http(s)://…`.
    pub fn new(settings: WebhookSettings) -> Result<Self, NotifyError> {
        let url = reqwest::Url::parse(&settings.url)
            .map_err(|e| NotifyError::Rejected(format!("invalid webhook URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(NotifyError::Rejected(
                "webhook URL must start with http:// or https://".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(WEBHOOK_TIMEOUT)
            .build()
            .map_err(|e| NotifyError::Rejected(e.to_string()))?;
        Ok(Self { client, settings })
    }

    fn request(&self, message: &Message) -> reqwest::RequestBuilder {
        let request = self.client.post(&self.settings.url);
        let request = match &self.settings.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        match self.settings.format {
            WebhookFormat::Json => request.json(&serde_json::json!({
                "source": SOURCE,
                "title": message.subject,
                "message": message.body,
            })),
            WebhookFormat::Text => request
                .header("Title", header_safe(&message.subject))
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(message.body.clone()),
        }
    }
}

/// HTTP headers only allow visible ASCII; umlauts are spelled out, the rest dropped.
fn header_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            'ä' => safe.push_str("ae"),
            'ö' => safe.push_str("oe"),
            'ü' => safe.push_str("ue"),
            'Ä' => safe.push_str("Ae"),
            'Ö' => safe.push_str("Oe"),
            'Ü' => safe.push_str("Ue"),
            'ß' => safe.push_str("ss"),
            c if c == ' ' || c.is_ascii_graphic() => safe.push(c),
            _ => {}
        }
    }
    safe
}

fn classify(status: StatusCode) -> Result<(), NotifyError> {
    if status.is_success() {
        Ok(())
    } else if status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
    {
        Err(NotifyError::Unavailable(format!(
            "webhook answered {status}"
        )))
    } else {
        Err(NotifyError::Rejected(format!("webhook answered {status}")))
    }
}

impl Notifier for WebhookNotifier {
    fn channel(&self) -> &'static str {
        "webhook"
    }

    async fn send(&self, message: &Message) -> Result<(), NotifyError> {
        let response = self
            .request(message)
            .send()
            .await
            .map_err(|e| NotifyError::Unavailable(e.to_string()))?;
        classify(response.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_are_accepted() {
        let settings = |url: &str| WebhookSettings {
            url: url.into(),
            format: WebhookFormat::Json,
            token: None,
        };
        assert!(WebhookNotifier::new(settings("https://ntfy.sh/topic")).is_ok());
        assert!(WebhookNotifier::new(settings("ftp://example.org")).is_err());
        assert!(WebhookNotifier::new(settings("no url")).is_err());
    }

    #[test]
    fn status_codes_are_classified() {
        assert!(classify(StatusCode::OK).is_ok());
        assert!(
            classify(StatusCode::BAD_GATEWAY)
                .unwrap_err()
                .is_temporary()
        );
        assert!(
            classify(StatusCode::TOO_MANY_REQUESTS)
                .unwrap_err()
                .is_temporary()
        );
        assert!(
            !classify(StatusCode::UNAUTHORIZED)
                .unwrap_err()
                .is_temporary()
        );
    }

    #[test]
    fn titles_are_made_header_safe() {
        assert_eq!(
            header_safe("Konflikt: Bericht.pdf"),
            "Konflikt: Bericht.pdf"
        );
        assert_eq!(header_safe("Wochenübersicht\n✓"), "Wochenuebersicht");
    }

    #[test]
    fn debug_output_hides_the_token() {
        let settings = WebhookSettings {
            url: "https://example.org".into(),
            format: WebhookFormat::Text,
            token: Some("tk_secret".into()),
        };
        assert!(!format!("{settings:?}").contains("tk_secret"));
    }
}

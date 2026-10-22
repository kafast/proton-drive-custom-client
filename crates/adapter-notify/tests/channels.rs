//! The channels against small fake servers on localhost (no real mail server or webhook).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use protonsync_adapter_notify::{
    MailNotifier, MailSecurity, MailSettings, WebhookFormat, WebhookNotifier, WebhookSettings,
};
use protonsync_application::ports::{Message, Notifier};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

fn message() -> Message {
    Message {
        subject: "Konflikt in main".into(),
        body: "Bericht.pdf wartet auf deine Entscheidung.".into(),
    }
}

/// Accepts one HTTP request, answers with `status` and returns the raw request.
async fn http_server(status: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        let response =
            format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        reader
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
        head + &String::from_utf8(body).unwrap()
    });
    (url, handle)
}

#[tokio::test]
async fn webhook_posts_json() {
    let (url, server) = http_server("200 OK").await;
    let notifier = WebhookNotifier::new(WebhookSettings {
        url,
        format: WebhookFormat::Json,
        token: Some("tk".into()),
    })
    .unwrap();

    notifier.send(&message()).await.unwrap();

    let request = server.await.unwrap();
    assert!(request.starts_with("POST /hook"), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer tk")
    );
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["source"], "protonsync");
    assert_eq!(json["title"], "Konflikt in main");
    assert_eq!(
        json["message"],
        "Bericht.pdf wartet auf deine Entscheidung."
    );
}

#[tokio::test]
async fn webhook_posts_text_with_title_header() {
    let (url, server) = http_server("200 OK").await;
    let notifier = WebhookNotifier::new(WebhookSettings {
        url,
        format: WebhookFormat::Text,
        token: None,
    })
    .unwrap();

    notifier.send(&message()).await.unwrap();

    let request = server.await.unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("title: konflikt in main")
    );
    assert!(request.ends_with("Bericht.pdf wartet auf deine Entscheidung."));
}

#[tokio::test]
async fn webhook_errors_say_whether_retrying_helps() {
    let (url, server) = http_server("503 Service Unavailable").await;
    let settings = WebhookSettings {
        url,
        format: WebhookFormat::Json,
        token: None,
    };
    let error = WebhookNotifier::new(settings.clone())
        .unwrap()
        .send(&message())
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(error.is_temporary());

    let (url, server) = http_server("401 Unauthorized").await;
    let error = WebhookNotifier::new(WebhookSettings { url, ..settings })
        .unwrap()
        .send(&message())
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(!error.is_temporary());
}

/// Minimal SMTP server without TLS and login: accepts one email and returns its data.
async fn smtp_server() -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        reader
            .get_mut()
            .write_all(b"220 fake ESMTP\r\n")
            .await
            .unwrap();
        let mut data = String::new();
        let mut in_data = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            if in_data {
                if line == ".\r\n" {
                    in_data = false;
                    reader.get_mut().write_all(b"250 queued\r\n").await.unwrap();
                } else {
                    data.push_str(&line);
                }
                continue;
            }
            let command = line.to_ascii_uppercase();
            let reply: &[u8] = if command.starts_with("EHLO") {
                b"250 fake\r\n"
            } else if command.starts_with("DATA") {
                in_data = true;
                b"354 go ahead\r\n"
            } else if command.starts_with("QUIT") {
                reader.get_mut().write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                data.push_str(&line);
                b"250 ok\r\n"
            };
            reader.get_mut().write_all(reply).await.unwrap();
        }
        data
    });
    (port, handle)
}

#[tokio::test]
async fn mail_is_delivered_over_smtp() {
    let (port, server) = smtp_server().await;
    let notifier = MailNotifier::new(&MailSettings {
        host: "127.0.0.1".into(),
        port,
        security: MailSecurity::None,
        login: None,
        from: "protonsync <sync@example.org>".into(),
        to: vec!["me@example.org".into(), "you@example.org".into()],
    })
    .unwrap();

    notifier.send(&message()).await.unwrap();
    drop(notifier);

    let data = server.await.unwrap();
    assert!(data.contains("MAIL FROM:<sync@example.org>"), "{data}");
    assert!(data.contains("RCPT TO:<me@example.org>"), "{data}");
    assert!(data.contains("RCPT TO:<you@example.org>"), "{data}");
    assert!(data.contains("Subject: Konflikt in main"), "{data}");
    assert!(
        data.contains("Bericht.pdf wartet auf deine Entscheidung."),
        "{data}"
    );
}

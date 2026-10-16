//! Web-Oberfläche am echten Daemon-Kern (mit Fakes) über eine echte
//! TCP-Verbindung: Start, Anmeldung, Statusseite, Beenden.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use protonsync_daemon::local_time::LocalZone;
use protonsync_daemon::web::web_config;
use protonsync_daemon::web_config::WebSettings;
use protonsync_web::{IpNetwork, PasswordCheck, hash_password};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

const PASSWORD: &str = "richtig-geheim";
const CONNECT_RETRIES: u32 = 50;
const RETRY_PAUSE: Duration = Duration::from_millis(20);

fn free_port() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

/// Schickt eine HTTP/1.1-Anfrage und liefert die ganze Antwort als Text.
async fn http(address: SocketAddr, request: &str) -> String {
    let mut stream = None;
    for _ in 0..CONNECT_RETRIES {
        if let Ok(connected) = TcpStream::connect(address).await {
            stream = Some(connected);
            break;
        }
        tokio::time::sleep(RETRY_PAUSE).await;
    }
    let Some(mut stream) = stream else {
        panic!("Web-Oberfläche nicht erreichbar");
    };
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

fn session_cookie(response: &str) -> String {
    response
        .lines()
        .find_map(|line| line.strip_prefix("set-cookie: "))
        .and_then(|cookie| cookie.split(';').next())
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn daemon_core_serves_the_web_interface() {
    let f = common::logged_in().await;
    let core = Arc::new(f.core);
    let address = free_port();
    let settings = WebSettings {
        bind: address,
        password: PasswordCheck::new(&hash_password(PASSWORD).unwrap()).unwrap(),
        allowed: vec!["127.0.0.0/8".parse::<IpNetwork>().unwrap()],
        tls: None,
        session_lifetime: Duration::from_hours(1),
    };
    let (stop, shutdown) = watch::channel(false);
    // `tokio::spawn` verlangt eine `Send`-Zukunft: Das prüft auch den Einbau.
    let server = tokio::spawn(protonsync_web::serve(
        web_config(&settings, &LocalZone::utc()),
        core,
        shutdown,
    ));

    let form = format!("password={PASSWORD}");
    let login = http(
        address,
        &format!(
            "POST /login HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
             Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{form}",
            form.len()
        ),
    )
    .await;
    assert!(login.starts_with("HTTP/1.1 303"), "{login}");
    let cookie = session_cookie(&login);

    let overview = http(
        address,
        &format!("GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\nCookie: {cookie}\r\n\r\n"),
    )
    .await;
    assert!(overview.starts_with("HTTP/1.1 200"), "{overview}");
    assert!(
        overview.contains(common::PAIR),
        "Sync-Paar sichtbar: {overview}"
    );

    stop.send(true).unwrap();
    server.await.unwrap().unwrap();
}

//! Socket-Server und Zeitplan mit dem Kern gegen Fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{T0, logged_in};
use protonsync_api::wire::{decode, encode};
use protonsync_api::{ClientMessage, ErrorCode, Event, Request, Response, ServerMessage};
use protonsync_application::ports::RemoteStorage;
use protonsync_daemon::config::Timing;
use protonsync_daemon::{scheduler, server};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;

struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    async fn connect(path: &std::path::Path) -> Self {
        let (read, writer) = UnixStream::connect(path).await.unwrap().into_split();
        Self {
            reader: BufReader::new(read),
            writer,
        }
    }

    async fn send_line(&mut self, line: &str) -> ServerMessage {
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.receive().await
    }

    async fn request(&mut self, request: Request) -> Response {
        let line = encode(&ClientMessage::new(request)).unwrap();
        match self.send_line(&line).await {
            ServerMessage::Response(response) => response,
            other @ ServerMessage::Event(_) => panic!("Antwort erwartet: {other:?}"),
        }
    }

    async fn receive(&mut self) -> ServerMessage {
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        decode(&line).unwrap()
    }
}

#[tokio::test]
async fn socket_answers_requests_and_streams_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run").join("daemon.sock");
    let f = logged_in().await;
    let core = Arc::new(f.core);
    let listener = server::bind(&path).await.unwrap();
    let (stop, shutdown) = watch::channel(false);
    let serving = tokio::spawn(server::serve(listener, Arc::clone(&core), shutdown));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(server::bind(&path).await.is_err(), "zweiter Daemon");

    let mut client = Client::connect(&path).await;
    assert!(matches!(
        client.request(Request::Status).await,
        Response::Status(_)
    ));

    let reply = client.send_line("kein json\n").await;
    let ServerMessage::Response(Response::Error(error)) = reply else {
        panic!("Fehler erwartet");
    };
    assert_eq!(error.code, ErrorCode::InvalidRequest);

    let reply = client
        .send_line("{\"version\":99,\"request\":{\"type\":\"status\"}}\n")
        .await;
    let ServerMessage::Response(Response::Error(error)) = reply else {
        panic!("Fehler erwartet");
    };
    assert_eq!(error.code, ErrorCode::VersionMismatch);

    let mut subscriber = Client::connect(&path).await;
    assert_eq!(subscriber.request(Request::Subscribe).await, Response::Done);
    client
        .request(Request::Sync {
            pair: None,
            apply: true,
        })
        .await;
    assert!(matches!(
        subscriber.receive().await,
        ServerMessage::Event(Event::SyncStarted { .. })
    ));

    stop.send(true).unwrap();
    serving.await.unwrap();
}

#[tokio::test]
async fn stale_socket_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.sock");
    std::fs::write(&path, b"").unwrap();
    assert!(server::bind(&path).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn scheduler_syncs_at_start_and_after_local_changes() {
    let f = logged_in().await;
    f.local.write("start.txt", b"1", T0);
    let core = Arc::new(f.core);
    let timing = Timing {
        event_poll: Duration::from_secs(60),
        full_sync: Duration::from_secs(3600),
        local_debounce: Duration::from_secs(3),
    };
    let (stop, shutdown) = watch::channel(false);
    let running = tokio::spawn(scheduler::run(Arc::clone(&core), timing, shutdown));

    tokio::time::sleep(Duration::from_secs(1)).await;
    let root = f.remote.root_folder().await.unwrap();
    assert_eq!(
        f.remote.list_children(&root.id).await.unwrap().len(),
        1,
        "Startabgleich"
    );

    f.local.write("neu.txt", b"2", T0);
    core.trigger(0).unwrap().notify_one();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        f.remote.list_children(&root.id).await.unwrap().len(),
        1,
        "wartet noch"
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(f.remote.list_children(&root.id).await.unwrap().len(), 2);

    stop.send(true).unwrap();
    running.await.unwrap();
}

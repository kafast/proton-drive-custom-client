//! Ereignis-Abonnement: eine eigene Verbindung, über die der Daemon
//! Fortschritt, Konflikte und Fehler meldet, sobald sie passieren.

use std::path::Path;

use protonsync_api::{Event, Request, Response, ServerMessage};

use crate::transport::{ClientError, SocketTransport, Transport};

/// Eine abonnierte Verbindung. [`EventSubscription::next_event`] blockiert,
/// bis das nächste Ereignis kommt; gedacht für einen eigenen Thread.
pub struct EventSubscription {
    transport: SocketTransport,
}

impl EventSubscription {
    /// Verbindet und abonniert. Der Daemon bestätigt erst, wenn das
    /// Abonnement steht; danach geht kein Ereignis verloren.
    ///
    /// # Errors
    /// [`ClientError::Unreachable`] ohne Daemon, [`ClientError::Protocol`],
    /// wenn der Daemon das Abonnement ablehnt.
    pub fn connect(path: &Path) -> Result<Self, ClientError> {
        let mut transport = SocketTransport::connect(path)?;
        match transport.request(Request::Subscribe)? {
            Response::Done => Ok(Self { transport }),
            Response::Error(error) => Err(ClientError::Protocol(error.message)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Wartet auf das nächste Ereignis.
    ///
    /// # Errors
    /// [`ClientError::Io`], wenn der Daemon beendet wird oder die Verbindung
    /// abbricht; [`ClientError::Protocol`] bei unerwarteten Nachrichten.
    pub fn next_event(&mut self) -> Result<Event, ClientError> {
        match self.transport.read_message()? {
            ServerMessage::Event(event) => Ok(event),
            ServerMessage::Response(response) => {
                Err(ClientError::Protocol(format!("{response:?}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    /// Ein Mini-Daemon, der auf die erste Zeile `lines` der Reihe nach schickt.
    fn fake_daemon(lines: &'static [&'static str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert!(first.contains("subscribe"), "{first}");
            for line in lines {
                writer.write_all(line.as_bytes()).unwrap();
            }
        });
        (dir, path)
    }

    #[test]
    fn delivers_events_until_the_daemon_stops() {
        let (_dir, path) = fake_daemon(&[
            "{\"kind\":\"response\",\"body\":{\"type\":\"done\"}}\n",
            "{\"kind\":\"event\",\"body\":{\"type\":\"sync_started\",\"pair\":\"p\"}}\n",
        ]);
        let mut events = EventSubscription::connect(&path).unwrap();
        assert_eq!(
            events.next_event().unwrap(),
            Event::SyncStarted { pair: "p".into() }
        );
        assert!(matches!(events.next_event(), Err(ClientError::Io(_))));
    }

    #[test]
    fn a_rejected_subscription_is_an_error() {
        let (_dir, path) = fake_daemon(&[
            "{\"kind\":\"response\",\"body\":{\"type\":\"error\",\"code\":\"version_mismatch\",\"message\":\"alt\"}}\n",
        ]);
        assert!(matches!(
            EventSubscription::connect(&path),
            Err(ClientError::Protocol(message)) if message == "alt"
        ));
    }

    #[test]
    fn responses_in_the_stream_are_unexpected() {
        let (_dir, path) = fake_daemon(&[
            "{\"kind\":\"response\",\"body\":{\"type\":\"done\"}}\n",
            "{\"kind\":\"response\",\"body\":{\"type\":\"done\"}}\n",
        ]);
        let mut events = EventSubscription::connect(&path).unwrap();
        assert!(matches!(events.next_event(), Err(ClientError::Protocol(_))));
    }
}

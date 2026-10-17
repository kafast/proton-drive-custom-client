//! Event subscription: a separate connection over which the daemon reports
//! progress, conflicts and errors as soon as they happen.

use std::path::Path;

use protonsync_api::{Event, Request, Response, ServerMessage};

use crate::transport::{ClientError, SocketTransport, Transport};

/// A subscribed connection. [`EventSubscription::next_event`] blocks until
/// the next event arrives; meant for a dedicated thread.
pub struct EventSubscription {
    transport: SocketTransport,
}

impl EventSubscription {
    /// Connects and subscribes. The daemon only confirms once the
    /// subscription is in place; after that no event is lost.
    ///
    /// # Errors
    /// [`ClientError::Unreachable`] without a daemon, [`ClientError::Protocol`]
    /// if the daemon rejects the subscription.
    pub fn connect(path: &Path) -> Result<Self, ClientError> {
        let mut transport = SocketTransport::connect(path)?;
        match transport.request(Request::Subscribe)? {
            Response::Done => Ok(Self { transport }),
            Response::Error(error) => Err(ClientError::Protocol(error.message)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Waits for the next event.
    ///
    /// # Errors
    /// [`ClientError::Io`] if the daemon stops or the connection drops;
    /// [`ClientError::Protocol`] on unexpected messages.
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

    /// A mini daemon that answers the first line by sending `lines` in order.
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
            "{\"kind\":\"response\",\"body\":{\"type\":\"error\",\"code\":\"version_mismatch\",\"message\":\"old\"}}\n",
        ]);
        assert!(matches!(
            EventSubscription::connect(&path),
            Err(ClientError::Protocol(message)) if message == "old"
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

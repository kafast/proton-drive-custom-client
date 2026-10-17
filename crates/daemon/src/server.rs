//! Unix socket server of the daemon API.
//!
//! The socket is readable and writable only by the own user (0600, in a
//! directory with 0700); other users of the machine cannot reach the daemon.

use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;

use protonsync_api::wire::{MAX_LINE_BYTES, decode, encode};
use protonsync_api::{
    ApiError, ClientMessage, ErrorCode, Event, PROTOCOL_VERSION, Request, Response, ServerMessage,
};
use protonsync_application::ports::{Account, Clock, LocalStorage};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, watch};

use crate::core::{Core, DaemonState};

#[cfg(unix)]
const SOCKET_DIR_MODE: u32 = 0o700;
#[cfg(unix)]
const SOCKET_MODE: u32 = 0o600;

/// What the server needs to answer (implemented by the core, replaceable in tests).
pub trait Handler: Send + Sync + 'static {
    /// Answers a request.
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send;
    /// Subscribes to events.
    fn subscribe(&self) -> broadcast::Receiver<Event>;
}

impl<A, L, S, C> Handler for Core<A, L, S, C>
where
    A: Account + 'static,
    A::Remote: 'static,
    L: LocalStorage + 'static,
    S: DaemonState,
    C: Clock + 'static,
{
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send {
        Core::handle(self, request)
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        Core::subscribe(self)
    }
}

/// Opens the socket. An orphaned socket of a crashed daemon is removed;
/// if a daemon is already running, opening fails.
///
/// # Errors
/// `AddrInUse` if a daemon is already running, otherwise file system errors.
pub async fn bind(path: &Path) -> io::Result<UnixListener> {
    // A relative name without a directory has the empty path as its parent.
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(SOCKET_DIR_MODE))?;
        }
    }
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("daemon is already running ({})", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    }
    Ok(listener)
}

/// Accepts connections until `shutdown` changes to `true`.
pub async fn serve<H: Handler>(
    listener: UnixListener,
    handler: Arc<H>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let handler = Arc::clone(&handler);
                    tokio::spawn(async move {
                        if let Err(error) = connection(stream, handler).await {
                            tracing::debug!(%error, "connection closed");
                        }
                    });
                }
                Err(error) => tracing::warn!(%error, "connection not accepted"),
            },
        }
    }
}

async fn connection<H: Handler>(stream: UnixStream, handler: Arc<H>) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    while let Some(line) = read_line_limited(&mut reader, MAX_LINE_BYTES).await? {
        let response = match decode::<ClientMessage>(&line) {
            Ok(message) if message.version != PROTOCOL_VERSION => Response::Error(ApiError::new(
                ErrorCode::VersionMismatch,
                format!(
                    "protocol version {} not supported (daemon: {PROTOCOL_VERSION})",
                    message.version
                ),
            )),
            Ok(ClientMessage {
                request: Request::Subscribe,
                ..
            }) => {
                // Subscribe first, then confirm: this way no event is lost.
                let events = handler.subscribe();
                send(&mut write, &ServerMessage::Response(Response::Done)).await?;
                return forward_events(events, &mut write).await;
            }
            Ok(message) => handler.handle(message.request).await,
            Err(error) => {
                Response::Error(ApiError::new(ErrorCode::InvalidRequest, error.to_string()))
            }
        };
        send(&mut write, &ServerMessage::Response(response)).await?;
    }
    Ok(())
}

async fn forward_events<W: AsyncWrite + Unpin>(
    mut events: broadcast::Receiver<Event>,
    write: &mut W,
) -> io::Result<()> {
    loop {
        match events.recv().await {
            Ok(event) => send(write, &ServerMessage::Event(event)).await?,
            // A subscriber that is too slow misses events but stays connected.
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

async fn send<W: AsyncWrite + Unpin>(write: &mut W, message: &ServerMessage) -> io::Result<()> {
    let line = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write.write_all(line.as_bytes()).await?;
    write.flush().await
}

/// Reads a line without `\n`; `None` at the end of the connection. Lines over
/// `max` bytes abort, so that a client cannot take up arbitrary memory.
async fn read_line_limited<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(end) = available.iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&available[..end]);
            reader.consume(end + 1);
            break;
        }
        let length = available.len();
        line.extend_from_slice(available);
        reader.consume(length);
        if line.len() > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message too large",
            ));
        }
    }
    if line.len() > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_lines_with_limit() {
        let mut input = &b"one\ntwo\nrest"[..];
        assert_eq!(
            read_line_limited(&mut input, 10).await.unwrap().as_deref(),
            Some("one")
        );
        assert_eq!(
            read_line_limited(&mut input, 10).await.unwrap().as_deref(),
            Some("two")
        );
        assert_eq!(
            read_line_limited(&mut input, 10).await.unwrap().as_deref(),
            Some("rest")
        );
        assert_eq!(read_line_limited(&mut input, 10).await.unwrap(), None);

        let mut long = &b"0123456789abc\n"[..];
        assert!(read_line_limited(&mut long, 10).await.is_err());
        let mut invalid = &[0xff, b'\n'][..];
        assert!(read_line_limited(&mut invalid, 10).await.is_err());
    }
}

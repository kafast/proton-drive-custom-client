//! Verbindung zum Daemon.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use protonsync_api::wire::{decode, encode};
use protonsync_api::{ClientMessage, Request, Response, ServerMessage};
use thiserror::Error;

/// Fehler der Verbindung.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Kein Daemon am Socket.
    #[error("{path}: {message}")]
    Unreachable {
        /// Socket-Pfad.
        path: PathBuf,
        /// Meldung des Betriebssystems.
        message: String,
    },
    /// Verbindung abgebrochen.
    #[error("Verbindung zum Daemon unterbrochen: {0}")]
    Io(String),
    /// Unerwartete Nachricht.
    #[error("Unerwartete Antwort des Daemons: {0}")]
    Protocol(String),
}

/// Schickt Anfragen und liefert Antworten (im Test ersetzbar).
pub trait Transport {
    /// Eine Anfrage, eine Antwort.
    ///
    /// # Errors
    /// [`ClientError`] bei Verbindungs- oder Protokollfehlern.
    fn request(&mut self, request: Request) -> Result<Response, ClientError>;
}

/// Verbindung über den Unix-Socket.
pub struct SocketTransport {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl SocketTransport {
    /// Verbindet mit dem Daemon.
    ///
    /// # Errors
    /// [`ClientError::Unreachable`], wenn dort kein Daemon lauscht.
    pub fn connect(path: &Path) -> Result<Self, ClientError> {
        let unreachable = |e: std::io::Error| ClientError::Unreachable {
            path: path.to_path_buf(),
            message: e.to_string(),
        };
        let writer = UnixStream::connect(path).map_err(unreachable)?;
        let reader = BufReader::new(writer.try_clone().map_err(unreachable)?);
        Ok(Self { reader, writer })
    }
}

impl Transport for SocketTransport {
    fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        let line = encode(&ClientMessage::new(request))
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| ClientError::Io(e.to_string()))?;
        let mut answer = String::new();
        let read = self
            .reader
            .read_line(&mut answer)
            .map_err(|e| ClientError::Io(e.to_string()))?;
        if read == 0 {
            return Err(ClientError::Io(
                "Daemon hat die Verbindung geschlossen".into(),
            ));
        }
        match decode::<ServerMessage>(&answer).map_err(|e| ClientError::Protocol(e.to_string()))? {
            ServerMessage::Response(response) => Ok(response),
            ServerMessage::Event(event) => Err(ClientError::Protocol(format!("{event:?}"))),
        }
    }
}

//! Connection to a running helper: sends requests, pairs the responses.
//!
//! One background task reads the helper's output line by line and hands each
//! response to the waiting caller (by its `id`). Several calls may be in
//! flight at the same time; the writer is protected by an async mutex so the
//! lines never interleave.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protonsync_application::ports::RemoteError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

use crate::protocol::{Hello, PROTOCOL_VERSION, Request, Response};

/// How long a single call may take before it counts as failed. Generous,
/// because one call transfers at most one chunk.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Callers waiting for an answer. A `std` mutex is enough: it is never held
/// across an `.await`.
type Pending = Arc<Mutex<PendingCalls>>;

/// Waiting callers by request id, and whether the helper has stopped answering.
#[derive(Default)]
struct PendingCalls {
    waiting: HashMap<u64, oneshot::Sender<Result<Value, RemoteError>>>,
    /// Set by the reader task when the helper's output ended.
    closed: bool,
}

/// The writing half, boxed so that a child process and an in-memory pipe
/// (tests) look the same.
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// A connection to the helper.
pub struct Connection {
    writer: tokio::sync::Mutex<Writer>,
    pending: Pending,
    next_id: AtomicU64,
    /// Keeps the child process alive as long as the connection exists;
    /// `kill_on_drop` ends it afterwards.
    _child: Option<Child>,
}

impl Connection {
    /// Connects over any pair of streams (e.g. an in-memory pipe in tests).
    /// Must be called inside a Tokio runtime because it starts the reader task.
    pub fn over<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self::with_child(reader, Box::new(writer), None)
    }

    /// Starts the helper program and connects to its stdin/stdout. Its
    /// stderr stays connected to ours, so its log lines land in our log.
    ///
    /// # Errors
    /// [`RemoteError::Unavailable`] if the program cannot be started.
    pub fn spawn(command: &mut Command) -> Result<Self, RemoteError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| RemoteError::Unavailable(format!("cannot start SDK helper: {e}")))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(RemoteError::Unavailable(
                "SDK helper has no stdin/stdout".into(),
            ));
        };
        Ok(Self::with_child(stdout, Box::new(stdin), Some(child)))
    }

    fn with_child<R>(reader: R, writer: Writer, child: Option<Child>) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let pending: Pending = Arc::default();
        tokio::spawn(read_responses(reader, Arc::clone(&pending)));
        Self {
            writer: tokio::sync::Mutex::new(writer),
            pending,
            next_id: AtomicU64::new(1),
            _child: child,
        }
    }

    /// Checks that the helper speaks our protocol; returns its SDK version.
    ///
    /// # Errors
    /// [`RemoteError::Rejected`] on a different protocol version, otherwise
    /// the error of the call.
    pub async fn hello(&self) -> Result<String, RemoteError> {
        let hello: Hello = self
            .call(
                "hello",
                &serde_json::json!({ "protocol": PROTOCOL_VERSION }),
            )
            .await?;
        if hello.protocol != PROTOCOL_VERSION {
            return Err(RemoteError::Rejected(format!(
                "SDK helper speaks protocol {}, expected {PROTOCOL_VERSION}",
                hello.protocol
            )));
        }
        Ok(hello.sdk_version)
    }

    /// Sends one request and waits for its response.
    ///
    /// # Errors
    /// The error the helper reports, [`RemoteError::Unavailable`] if the
    /// helper is gone or too slow, [`RemoteError::Transfer`] if the answer
    /// does not have the expected shape.
    pub async fn call<P, T>(&self, method: &str, params: &P) -> Result<T, RemoteError>
    where
        P: Serialize + Sync,
        T: DeserializeOwned,
    {
        let value = self.call_raw(method, params).await?;
        serde_json::from_value(value)
            .map_err(|e| RemoteError::Transfer(format!("unexpected answer to {method}: {e}")))
    }

    async fn call_raw<P: Serialize + Sync>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<Value, RemoteError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::to_value(params)
            .map_err(|e| RemoteError::Transfer(format!("cannot encode {method}: {e}")))?;
        let mut line = serde_json::to_string(&Request {
            id,
            method: method.to_owned(),
            params,
        })
        .map_err(|e| RemoteError::Transfer(format!("cannot encode {method}: {e}")))?;
        line.push('\n');

        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = lock(&self.pending);
            if pending.closed {
                return Err(helper_gone());
            }
            pending.waiting.insert(id, sender);
        }
        if let Err(error) = self.write_line(&line).await {
            lock(&self.pending).waiting.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(CALL_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            // The reader task ended and dropped the sender.
            Ok(Err(_)) => Err(helper_gone()),
            Err(_) => {
                lock(&self.pending).waiting.remove(&id);
                Err(RemoteError::Unavailable(format!(
                    "SDK helper did not answer {method} in time"
                )))
            }
        }
    }

    async fn write_line(&self, line: &str) -> Result<(), RemoteError> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| helper_gone())?;
        writer.flush().await.map_err(|_| helper_gone())
    }
}

/// Reads responses until the helper closes its output, then fails all
/// callers that are still waiting.
async fn read_responses<R: AsyncRead + Unpin>(reader: R, pending: Pending) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(response) = serde_json::from_str::<Response>(&line) else {
            // Not ours (e.g. a stray print in the helper); ignore it.
            continue;
        };
        let result = match response.error {
            Some(error) => Err(error.into_remote_error()),
            // `"ok": null` arrives as `None`; that is a successful empty answer.
            None => Ok(response.ok.unwrap_or(Value::Null)),
        };
        if let Some(sender) = lock(&pending).waiting.remove(&response.id) {
            // The caller may have given up (timeout); then nobody listens.
            let _ = sender.send(result);
        }
    }
    let mut pending = lock(&pending);
    pending.closed = true;
    for (_, sender) in pending.waiting.drain() {
        let _ = sender.send(Err(helper_gone()));
    }
}

fn helper_gone() -> RemoteError {
    RemoteError::Unavailable("SDK helper is not running".into())
}

/// Locks the map. A poisoned lock (a panic elsewhere) is taken over anyway:
/// the map itself is always consistent.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

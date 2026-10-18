//! HTTP or HTTPS server of the interface.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tower::ServiceExt;

use crate::visitor::ClientAddr;
use crate::{Backend, TlsFiles, WebConfig, router};

/// How long a client may take for the headers of its request.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the TLS handshake may take.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors when starting the interface.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// Address not usable (in use, no permission).
    #[error("web interface cannot open {address}: {message}")]
    Bind {
        /// Address.
        address: String,
        /// Message.
        message: String,
    },
    /// Certificate or key not readable or invalid.
    #[error("TLS files unusable: {0}")]
    Tls(String),
}

/// Loads the certificate chain and key (PEM).
fn tls_acceptor(files: &TlsFiles) -> Result<TlsAcceptor, WebError> {
    let certs = CertificateDer::pem_file_iter(&files.cert)
        .map_err(|e| WebError::Tls(format!("{}: {e}", files.cert.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| WebError::Tls(format!("{}: {e}", files.cert.display())))?;
    if certs.is_empty() {
        return Err(WebError::Tls(format!(
            "{}: no certificate found",
            files.cert.display()
        )));
    }
    let key = PrivateKeyDer::from_pem_file(&files.key)
        .map_err(|e| WebError::Tls(format!("{}: {e}", files.key.display())))?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| WebError::Tls(e.to_string()))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Starts the interface and runs until `shutdown` switches to `true`.
///
/// Deliberately not an `async fn`: the routes are built right away, so the
/// returned future no longer depends on the type `B`. Otherwise the compiler
/// (as of Rust 1.99) cannot prove that it is `Send`, and `tokio::spawn`
/// rejects it.
///
/// # Errors
/// [`WebError`] if the address or the TLS files are not usable.
pub fn serve<B: Backend>(
    config: WebConfig,
    backend: Arc<B>,
    shutdown: watch::Receiver<bool>,
) -> impl Future<Output = Result<(), WebError>> + Send {
    let app = router(backend, &config);
    serve_router(config, app, shutdown)
}

async fn serve_router(
    config: WebConfig,
    app: axum::Router,
    shutdown: watch::Receiver<bool>,
) -> Result<(), WebError> {
    let tls = config.tls.as_ref().map(tls_acceptor).transpose()?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|e| WebError::Bind {
            address: config.bind.to_string(),
            message: e.to_string(),
        })?;
    let address = listener
        .local_addr()
        .map_or_else(|_| config.bind.to_string(), |a| a.to_string());
    tracing::info!(
        %address,
        https = tls.is_some(),
        "web interface ready"
    );
    if tls.is_none() && !config.bind.ip().is_loopback() {
        tracing::warn!(
            "web interface without TLS on the network: password and contents travel unencrypted \
             over the network. Set up TLS or put a reverse proxy with HTTPS in front."
        );
    }
    accept_loop(listener, tls, app, shutdown).await;
    Ok(())
}

/// Accepts connections until `shutdown` switches to `true`.
async fn accept_loop(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    app: axum::Router,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut connections = JoinSet::new();
    loop {
        let (stream, peer) = tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "connection not accepted");
                    continue;
                }
            },
            // Clean up finished connections so the list does not grow.
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
        };
        let app = app.clone();
        let tls = tls.clone();
        connections.spawn(async move {
            let result = match tls {
                Some(acceptor) => {
                    match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
                    {
                        Ok(Ok(stream)) => serve_connection(stream, peer, app).await,
                        Ok(Err(error)) => Err(error),
                        Err(_) => Err(io::Error::from(io::ErrorKind::TimedOut)),
                    }
                }
                None => serve_connection(stream, peer, app).await,
            };
            if let Err(error) = result {
                tracing::debug!(%peer, %error, "connection closed");
            }
        });
    }
    connections.shutdown().await;
    tracing::info!("web interface stopped");
}
async fn serve_connection<S>(
    stream: S,
    peer: std::net::SocketAddr,
    app: axum::Router,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
        request.extensions_mut().insert(ClientAddr(peer));
        app.clone().oneshot(request)
    });
    http1::Builder::new()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT)
        .serve_connection(TokioIo::new(stream), service)
        .await
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tls_files_are_reported() {
        let files = TlsFiles {
            cert: "/does/not/exist.pem".into(),
            key: "/does/not/exist.key".into(),
        };
        let error = tls_acceptor(&files).err().unwrap();
        assert!(error.to_string().contains("/does/not/exist.pem"), "{error}");
    }
}

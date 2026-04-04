use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use async_stream::stream;
use futures::Stream;
use spiffe::X509Source;
use spiffe_rustls::authorizer;
use spiffe_rustls::mtls_server;
use spiffe_rustls_tokio::PeerIdentity;
use spiffe_rustls_tokio::TlsAcceptor;
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tonic::transport::server::Connected;
use tonic::transport::server::TcpConnectInfo;

use crate::config::SpiffeTransportConfig;

const SPIFFE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("SPIFFE identity bootstrap timed out")]
    SpiffeBootstrapTimeout,
    #[error("SPIFFE identity bootstrap failed: {0}")]
    SpiffeSource(#[from] spiffe::x509_source::X509SourceError),
    #[error("SPIFFE TLS configuration failed: {0}")]
    SpiffeTls(#[from] spiffe_rustls::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedPeerIdentity {
    spiffe_id: String,
}

impl AuthenticatedPeerIdentity {
    pub(crate) fn new(spiffe_id: impl Into<String>) -> Self {
        Self {
            spiffe_id: spiffe_id.into(),
        }
    }

    pub(crate) fn spiffe_id(&self) -> &str {
        &self.spiffe_id
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SpiffeConnectInfo {
    tcp: TcpConnectInfo,
    peer_identity: Option<AuthenticatedPeerIdentity>,
}

impl SpiffeConnectInfo {
    pub(crate) fn remote_addr(&self) -> Option<SocketAddr> {
        self.tcp.remote_addr()
    }

    pub(crate) fn peer_identity(&self) -> Option<&AuthenticatedPeerIdentity> {
        self.peer_identity.as_ref()
    }
}

pub(crate) async fn build_spiffe_incoming(
    listener: TcpListener,
    config: &SpiffeTransportConfig,
) -> Result<
    Pin<Box<dyn Stream<Item = Result<ConnectedSpiffeStream, std::io::Error>> + Send>>,
    TransportError,
> {
    let socket = config.workload_api_socket.as_deref().unwrap_or("<default>");
    tracing::debug!(
        allowed_client_id = config.allowed_client_id.as_str(),
        workload_api_socket = socket,
        "bootstrapping SPIFFE server identity"
    );
    let source = timeout(SPIFFE_BOOTSTRAP_TIMEOUT, build_x509_source(config))
        .await
        .map_err(|_| {
            tracing::warn!(
                workload_api_socket = socket,
                "SPIFFE server identity bootstrap timed out — \
                 check that the SPIRE agent is running and reachable at the configured socket, \
                 and that a workload entry is registered for this process"
            );
            TransportError::SpiffeBootstrapTimeout
        })??;
    tracing::debug!("SPIFFE server identity bootstrap succeeded");

    let tls_config = mtls_server(source)
        .authorize(authorizer::exact([config.allowed_client_id.as_str()])?)
        .with_alpn_protocols([b"h2"])
        .build()?;
    let acceptor = TlsAcceptor::new(Arc::new(tls_config));
    tracing::debug!("SPIFFE TLS acceptor ready");

    Ok(Box::pin(stream! {
        loop {
            let (tcp_stream, remote_addr) = match listener.accept().await {
                Ok((stream, remote_addr)) => (stream, remote_addr),
                Err(error) => {
                    tracing::warn!(error = %error, "failed to accept inbound SPIFFE TCP connection");
                    continue;
                }
            };

            match acceptor.accept(tcp_stream).await {
                Ok((tls_stream, peer_identity)) => {
                    log_peer_identity(&peer_identity, remote_addr);
                    let peer_identity = peer_identity
                        .spiffe_id()
                        .map(|spiffe_id| AuthenticatedPeerIdentity::new(spiffe_id.to_string()));
                    yield Ok(ConnectedSpiffeStream::new(tls_stream, remote_addr, peer_identity));
                }
                Err(error) => {
                    tracing::warn!(
                        remote_addr = %remote_addr,
                        error = %error,
                        "rejecting inbound SPIFFE TLS connection"
                    );
                }
            }
        }
    }))
}

async fn build_x509_source(config: &SpiffeTransportConfig) -> Result<X509Source, TransportError> {
    match &config.workload_api_socket {
        Some(socket) => Ok(X509Source::builder().endpoint(socket).build().await?),
        None => Ok(X509Source::new().await?),
    }
}

fn log_peer_identity(peer_identity: &PeerIdentity, remote_addr: SocketAddr) {
    if let Some(spiffe_id) = peer_identity.spiffe_id() {
        tracing::debug!(remote_addr = %remote_addr, spiffe_id = %spiffe_id, "accepted SPIFFE peer");
    } else {
        tracing::debug!(
            remote_addr = %remote_addr,
            "accepted SPIFFE peer without extracted SPIFFE ID"
        );
    }
}

pub(crate) struct ConnectedSpiffeStream {
    stream: tokio_rustls::server::TlsStream<TcpStream>,
    remote_addr: SocketAddr,
    peer_identity: Option<AuthenticatedPeerIdentity>,
}

impl ConnectedSpiffeStream {
    fn new(
        stream: tokio_rustls::server::TlsStream<TcpStream>,
        remote_addr: SocketAddr,
        peer_identity: Option<AuthenticatedPeerIdentity>,
    ) -> Self {
        Self {
            stream,
            remote_addr,
            peer_identity,
        }
    }
}

impl AsyncRead for ConnectedSpiffeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for ConnectedSpiffeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl Connected for ConnectedSpiffeStream {
    type ConnectInfo = SpiffeConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        SpiffeConnectInfo {
            tcp: TcpConnectInfo {
                local_addr: self.stream.get_ref().0.local_addr().ok(),
                remote_addr: Some(self.remote_addr),
            },
            peer_identity: self.peer_identity.clone(),
        }
    }
}

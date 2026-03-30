use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use http::Uri;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use spiffe::X509Source;
use spiffe_rustls::authorizer;
use spiffe_rustls::mtls_client;
use spiffe_rustls_tokio::TlsConnector;
use tokio::net::lookup_host;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tower::Service;

use crate::ClientError;
use crate::SpiffeMtlsConfig;

const SPIFFE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const SPIFFE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds a SPIFFE-backed tonic channel.
///
/// The `rustls::ClientConfig` is backed by a live `X509Source`, so new TLS handshakes continue to
/// pick up rotated SVIDs and trust bundles. This is more than a startup snapshot, even though the
/// current `anki-edit` usage is short-lived.
pub(crate) async fn connect_channel(
    endpoint: &str,
    config: &SpiffeMtlsConfig,
) -> Result<tonic::transport::Channel, ClientError> {
    let target = EndpointTarget::from_endpoint(endpoint)?;
    let source = timeout(SPIFFE_BOOTSTRAP_TIMEOUT, build_x509_source(config))
        .await
        .map_err(|_| ClientError::SpiffeBootstrapTimeout)??;

    let tls_config = mtls_client(source)
        .authorize(authorizer::exact([config.expected_server_id.as_str()])?)
        .with_alpn_protocols([b"h2"])
        .build()?;
    let connector = SpiffeConnector {
        host: target.host,
        port: target.port,
        server_name: target.server_name,
        tls_connector: TlsConnector::new(Arc::new(tls_config)),
    };

    target
        .endpoint
        .connect_with_connector(connector)
        .await
        .map_err(Into::into)
}

async fn build_x509_source(config: &SpiffeMtlsConfig) -> Result<X509Source, ClientError> {
    match &config.workload_api_socket {
        Some(socket) => Ok(X509Source::builder().endpoint(socket).build().await?),
        None => Ok(X509Source::new().await?),
    }
}

#[derive(Clone)]
struct SpiffeConnector {
    host: String,
    port: u16,
    server_name: ServerName<'static>,
    tls_connector: TlsConnector,
}

impl Service<Uri> for SpiffeConnector {
    type Response = TokioIo<tokio_rustls::client::TlsStream<TcpStream>>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Uri) -> Self::Future {
        let host = self.host.clone();
        let port = self.port;
        let server_name = self.server_name.clone();
        let tls_connector = self.tls_connector.clone();

        Box::pin(async move {
            // Tonic passes the logical endpoint URI here, but this connector owns transport
            // dialing and TLS policy, so it resolves and connects using the prevalidated host/port.
            let mut addresses = lookup_host((host.as_str(), port)).await?;
            let first_addr = addresses.next().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!("no addresses resolved for {host}:{port}"),
                )
            })?;
            let mut last_error = None;
            let mut tcp = None;

            for socket_addr in std::iter::once(first_addr).chain(addresses) {
                match TcpStream::connect(socket_addr).await {
                    Ok(stream) => {
                        tcp = Some(stream);
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }

            let tcp = tcp.ok_or_else(|| {
                last_error.unwrap_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        format!("no reachable addresses for {host}:{port}"),
                    )
                })
            })?;
            let (tls_stream, _peer_identity) = tls_connector
                .connect(server_name, tcp)
                .await
                .map_err(std::io::Error::other)?;
            Ok(TokioIo::new(tls_stream))
        })
    }
}

struct EndpointTarget {
    endpoint: tonic::transport::Endpoint,
    host: String,
    port: u16,
    server_name: ServerName<'static>,
}

impl EndpointTarget {
    fn from_endpoint(endpoint: &str) -> Result<Self, ClientError> {
        let endpoint_builder = tonic::transport::Endpoint::from_shared(endpoint.to_owned())
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?
            .connect_timeout(SPIFFE_CONNECT_TIMEOUT);
        let uri = Uri::from_str(endpoint)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let host = uri
            .host()
            .ok_or_else(|| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let port = uri.port_u16().unwrap_or(443);
        let server_name = server_name_from_host(host)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;

        Ok(Self {
            endpoint: endpoint_builder,
            host: host.to_owned(),
            port,
            server_name,
        })
    }
}

fn server_name_from_host(
    host: &str,
) -> Result<ServerName<'static>, rustls::pki_types::InvalidDnsNameError> {
    if let Ok(ip_addr) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip_addr.into()));
    }

    ServerName::try_from(host.to_owned())
}

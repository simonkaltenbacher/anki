use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use http::Uri;
use rustls::pki_types::ServerName;
use spiffe::X509Source;
use spiffe_rustls::authorizer;
use spiffe_rustls::mtls_client;
use tokio::time::timeout;

use crate::rustls_channel;
use crate::Channel;
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
) -> Result<Channel, ClientError> {
    let target = EndpointTarget::from_endpoint(endpoint)?;

    let socket = config.workload_api_socket.as_deref().unwrap_or("<default>");
    tracing::debug!(
        endpoint = endpoint,
        expected_server_id = config.expected_server_id.as_str(),
        workload_api_socket = socket,
        "bootstrapping SPIFFE identity"
    );
    let source = timeout(SPIFFE_BOOTSTRAP_TIMEOUT, build_x509_source(config))
        .await
        .map_err(|_| {
            tracing::warn!(
                workload_api_socket = socket,
                "SPIFFE identity bootstrap timed out — \
                 check that the SPIRE agent is running and reachable at the configured socket, \
                 and that a workload entry is registered for this process"
            );
            ClientError::SpiffeBootstrapTimeout
        })??;
    tracing::debug!("SPIFFE identity bootstrap succeeded");

    let tls_config = mtls_client(source)
        .authorize(authorizer::exact([config.expected_server_id.as_str()])?)
        .with_alpn_protocols([b"h2"])
        .build()?;

    tracing::debug!(endpoint = endpoint, "connecting SPIFFE mTLS channel");
    let channel = rustls_channel::connect(
        target.uri,
        Some(SPIFFE_CONNECT_TIMEOUT),
        tls_config,
        target.server_name,
    )
    .await
    .inspect_err(|error| {
        tracing::debug!(error = %error, "SPIFFE mTLS channel connection failed");
    })?;
    tracing::debug!("SPIFFE mTLS channel connected");
    Ok(Channel::Spiffe(channel))
}

async fn build_x509_source(config: &SpiffeMtlsConfig) -> Result<X509Source, ClientError> {
    match &config.workload_api_socket {
        Some(socket) => Ok(X509Source::builder().endpoint(socket).build().await?),
        None => Ok(X509Source::new().await?),
    }
}

struct EndpointTarget {
    uri: Uri,
    server_name: ServerName<'static>,
}

impl EndpointTarget {
    fn from_endpoint(endpoint: &str) -> Result<Self, ClientError> {
        let uri = Uri::from_str(endpoint)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let host = uri
            .host()
            .ok_or_else(|| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let server_name = server_name_from_host(host)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;

        Ok(Self { uri, server_name })
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

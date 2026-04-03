use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use http::Uri;
use rustls::pki_types::ServerName;
use spiffe::X509Source;
use spiffe_rustls::authorizer;
use spiffe_rustls::mtls_client;
use tokio::time::timeout;
use tonic::transport::ClientTlsConfig;
use tonic::transport::Endpoint;

use crate::channel;
use crate::Channel;
use crate::ClientError;
use crate::ConnectionConfig;
use crate::SpiffeMtlsConfig;
use crate::TransportConfig;

const SPIFFE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const SPIFFE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn connect(config: &ConnectionConfig) -> Result<Channel, ClientError> {
    let target = EndpointTarget::try_from(config.endpoint.as_str())?;

    match &config.transport {
        TransportConfig::Plaintext => connect_tonic_channel(&target, false).await,
        TransportConfig::Tls => connect_tonic_channel(&target, true).await,
        TransportConfig::SpiffeMtls(spiffe_config) => connect_spiffe(&target, spiffe_config).await,
    }
}

pub(super) struct EndpointTarget {
    pub(super) raw: String,
    pub(super) uri: Uri,
    pub(super) server_name: ServerName<'static>,
}

impl TryFrom<&str> for EndpointTarget {
    type Error = ClientError;

    fn try_from(endpoint: &str) -> Result<Self, Self::Error> {
        let uri = Uri::from_str(endpoint)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let host = uri
            .host()
            .ok_or_else(|| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
        let server_name = server_name_from_host(host)
            .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;

        Ok(Self {
            raw: endpoint.to_owned(),
            uri,
            server_name,
        })
    }
}

impl EndpointTarget {
    pub(super) fn to_tonic_endpoint(&self) -> Result<Endpoint, ClientError> {
        Endpoint::from_shared(self.raw.clone())
            .map_err(|_| ClientError::InvalidEndpoint(self.raw.clone()))
    }
}

async fn connect_tonic_channel(
    target: &EndpointTarget,
    use_tls: bool,
) -> Result<Channel, ClientError> {
    let mut endpoint = target.to_tonic_endpoint()?;
    if use_tls {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_enabled_roots())?;
    }

    endpoint
        .connect()
        .await
        .map(Channel::Tonic)
        .map_err(Into::into)
}

async fn connect_spiffe(
    target: &EndpointTarget,
    config: &SpiffeMtlsConfig,
) -> Result<Channel, ClientError> {
    let socket = config.workload_api_socket.as_deref().unwrap_or("<default>");
    tracing::debug!(
        endpoint = target.raw.as_str(),
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

    tracing::debug!(
        endpoint = target.raw.as_str(),
        "connecting SPIFFE mTLS channel"
    );
    let channel = channel::Channel::connect(
        target.uri.clone(),
        Some(SPIFFE_CONNECT_TIMEOUT),
        tls_config,
        target.server_name.clone(),
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

fn server_name_from_host(
    host: &str,
) -> Result<ServerName<'static>, rustls::pki_types::InvalidDnsNameError> {
    if let Ok(ip_addr) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip_addr.into()));
    }

    ServerName::try_from(host.to_owned())
}

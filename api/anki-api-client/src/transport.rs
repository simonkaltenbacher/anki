use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use http::Uri;
use rustls::pki_types::ServerName;
use rustls::RootCertStore;
use spiffe::X509Source;
use spiffe_rustls::authorizer;
use spiffe_rustls::mtls_client;
use tokio::time::timeout;

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
        TransportConfig::Plaintext => connect_plaintext(&target).await,
        TransportConfig::Tls => connect_tls(&target).await,
        TransportConfig::SpiffeMtls(spiffe_config) => connect_spiffe(&target, spiffe_config).await,
    }
}

struct EndpointTarget {
    raw: String,
    uri: Uri,
    server_name: ServerName<'static>,
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

async fn connect_plaintext(target: &EndpointTarget) -> Result<Channel, ClientError> {
    channel::Channel::connect(
        target.uri.clone(),
        None,
        channel::TransportSecurity::Plaintext,
    )
    .await
    .map_err(Into::into)
}

async fn connect_tls(target: &EndpointTarget) -> Result<Channel, ClientError> {
    let tls_config = build_tls_config()?;
    channel::Channel::connect(
        target.uri.clone(),
        None,
        channel::TransportSecurity::Tls {
            tls_config,
            server_name: target.server_name.clone(),
        },
    )
    .await
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
        channel::TransportSecurity::Tls {
            tls_config,
            server_name: target.server_name.clone(),
        },
    )
    .await
    .inspect_err(|error| {
        tracing::debug!(error = %error, "SPIFFE mTLS channel connection failed");
    })?;
    tracing::debug!("SPIFFE mTLS channel connected");
    Ok(channel)
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

fn build_tls_config() -> Result<tokio_rustls::rustls::ClientConfig, ClientError> {
    let native_certs = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    for cert in native_certs.certs {
        roots.add(cert).map_err(|error| {
            ClientError::TlsConfig(format!("failed to add native root certificate: {error}"))
        })?;
    }

    if roots.is_empty() {
        return Err(ClientError::TlsConfig(
            "no native root certificates available".to_owned(),
        ));
    }

    Ok(tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

use std::net::IpAddr;
use std::str::FromStr;

use http::Uri;
use rustls::pki_types::ServerName;
use tonic::transport::ClientTlsConfig;
use tonic::transport::Endpoint;

use crate::Channel;
use crate::ClientError;
use crate::ConnectionConfig;
use crate::TransportConfig;

mod spiffe;

pub(crate) async fn connect_channel(config: &ConnectionConfig) -> Result<Channel, ClientError> {
    let target = EndpointTarget::try_from(config.endpoint.as_str())?;

    match &config.transport {
        TransportConfig::Plaintext => connect_tonic_channel(&target, false).await,
        TransportConfig::Tls => connect_tonic_channel(&target, true).await,
        TransportConfig::SpiffeMtls(spiffe_config) => {
            spiffe::connect_channel(&target, spiffe_config).await
        }
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

fn server_name_from_host(
    host: &str,
) -> Result<ServerName<'static>, rustls::pki_types::InvalidDnsNameError> {
    if let Ok(ip_addr) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip_addr.into()));
    }

    ServerName::try_from(host.to_owned())
}

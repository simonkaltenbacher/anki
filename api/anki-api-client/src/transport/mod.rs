use std::net::IpAddr;
use std::str::FromStr;

use http::Uri;
use rustls::pki_types::ServerName;
use tonic::transport::Endpoint;

use crate::Channel;
use crate::ClientError;
use crate::ConnectionConfig;
use crate::TransportConfig;

mod plaintext;
mod spiffe;
mod tls;

pub(crate) async fn connect_channel(config: &ConnectionConfig) -> Result<Channel, ClientError> {
    let target = EndpointTarget::try_from(config.endpoint.as_str())?;

    match &config.transport {
        TransportConfig::Plaintext => plaintext::connect_channel(&target).await,
        TransportConfig::Tls => tls::connect_channel(&target).await,
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

fn server_name_from_host(
    host: &str,
) -> Result<ServerName<'static>, rustls::pki_types::InvalidDnsNameError> {
    if let Ok(ip_addr) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip_addr.into()));
    }

    ServerName::try_from(host.to_owned())
}

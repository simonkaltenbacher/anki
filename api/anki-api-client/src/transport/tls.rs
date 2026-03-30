use tonic::transport::ClientTlsConfig;
use tonic::transport::Endpoint;

use crate::ClientError;

pub(crate) async fn connect_channel(
    endpoint: &str,
) -> Result<tonic::transport::Channel, ClientError> {
    let endpoint = Endpoint::from_shared(endpoint.to_owned())
        .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?
        .tls_config(ClientTlsConfig::new().with_enabled_roots())?;
    endpoint.connect().await.map_err(Into::into)
}

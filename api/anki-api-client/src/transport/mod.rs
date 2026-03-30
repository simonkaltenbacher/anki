use crate::ClientError;
use crate::ConnectionConfig;
use crate::TransportConfig;

mod plaintext;
mod spiffe;

pub(crate) async fn connect_channel(
    config: &ConnectionConfig,
) -> Result<tonic::transport::Channel, ClientError> {
    match &config.transport {
        TransportConfig::Plaintext => plaintext::connect_channel(&config.endpoint).await,
        TransportConfig::SpiffeMtls(spiffe_config) => {
            spiffe::connect_channel(&config.endpoint, spiffe_config).await
        }
    }
}

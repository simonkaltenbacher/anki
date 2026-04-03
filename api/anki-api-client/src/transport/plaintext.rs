use crate::Channel;
use crate::ClientError;

pub(crate) async fn connect_channel(endpoint: &str) -> Result<Channel, ClientError> {
    let endpoint = tonic::transport::Endpoint::from_shared(endpoint.to_owned())
        .map_err(|_| ClientError::InvalidEndpoint(endpoint.to_owned()))?;
    endpoint
        .connect()
        .await
        .map(Channel::Tonic)
        .map_err(Into::into)
}

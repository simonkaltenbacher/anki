use super::EndpointTarget;
use crate::Channel;
use crate::ClientError;

pub(crate) async fn connect_channel(target: &EndpointTarget) -> Result<Channel, ClientError> {
    let endpoint = target.to_tonic_endpoint()?;
    endpoint
        .connect()
        .await
        .map(Channel::Tonic)
        .map_err(Into::into)
}

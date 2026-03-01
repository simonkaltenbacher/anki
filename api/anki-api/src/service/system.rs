use anki_api_proto::anki::api::v1::GetServerInfoRequest;
use anki_api_proto::anki::api::v1::GetServerInfoResponse;
use anki_api_proto::anki::api::v1::system_service_server::SystemService;
use tonic::Request;
use tonic::Response;
use tonic::Status;

#[derive(Clone, Debug)]
pub struct SystemApi {
    server_version: String,
    anki_version: Option<String>,
    capabilities: Vec<String>,
}

impl SystemApi {
    pub fn new(
        server_version: String,
        anki_version: Option<String>,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            server_version,
            anki_version,
            capabilities,
        }
    }
}

#[tonic::async_trait]
impl SystemService for SystemApi {
    async fn get_server_info(
        &self,
        _request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
        Ok(Response::new(GetServerInfoResponse {
            api_version: "v1".to_owned(),
            server_version: self.server_version.clone(),
            capabilities: self.capabilities.clone(),
            anki_version: self.anki_version.clone().unwrap_or_default(),
        }))
    }
}

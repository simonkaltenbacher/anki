use anki_api_proto::anki::api::v1::HealthCheckRequest;
use anki_api_proto::anki::api::v1::HealthCheckResponse;
use anki_api_proto::anki::api::v1::health_check_response::Status as HealthStatus;
use anki_api_proto::anki::api::v1::health_service_server::HealthService;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::store::SharedStore;

#[derive(Clone)]
pub struct HealthApi {
    store: SharedStore,
}

impl HealthApi {
    pub fn new(store: SharedStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl HealthService for HealthApi {
    async fn check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let status = if self.store.list_notetype_ids().is_ok() {
            HealthStatus::Serving
        } else {
            tracing::warn!("health probe failed backend check");
            HealthStatus::NotServing
        };
        Ok(Response::new(HealthCheckResponse {
            status: status as i32,
        }))
    }
}

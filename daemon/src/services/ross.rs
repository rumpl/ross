use ross_core::ross_server::Ross;
use ross_core::{HealthCheckRequest, HealthCheckResponse};
use tonic::{Request, Response, Status};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub struct RossService;

#[tonic::async_trait]
impl Ross for RossService {
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let response = HealthCheckResponse {
            healthy: true,
            version: VERSION.to_string(),
        };
        Ok(Response::new(response))
    }
}

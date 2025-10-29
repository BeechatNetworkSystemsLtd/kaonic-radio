use tonic::{Request, Response, Status};

use super::kaonic::{
    device_server::Device,
    Empty,
    InfoResponse,
    StatisticsResponse,
};

#[derive(Default)]
pub struct DeviceService;

#[tonic::async_trait]
impl Device for DeviceService {
    async fn get_info(&self, _request: Request<Empty>) -> Result<Response<InfoResponse>, Status> {
        // Stub: return empty response
        Ok(Response::new(InfoResponse {}))
    }

    async fn get_statistics(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<StatisticsResponse>, Status> {
        // Stub: return empty response
        Ok(Response::new(StatisticsResponse {}))
    }
}

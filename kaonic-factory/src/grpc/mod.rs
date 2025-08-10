pub mod factory;

use factory::{FactoryService, kaonic::factory_server::FactoryServer};
use tonic::transport::Server;

pub async fn start_server() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50011".parse()?;
    let factory_service = FactoryService::default();

    Server::builder()
        .add_service(FactoryServer::new(factory_service))
        .serve(addr)
        .await?;

    Ok(())
}

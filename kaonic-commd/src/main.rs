mod grpc;
mod radio_service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    simple_logger::SimpleLogger::new().env().init().ok();

    let version = env!("CARGO_PKG_VERSION");
    let addr = "0.0.0.0:8080".to_string();

    log::info!("Kaonic Communication Daemon: v{}", version);
    log::info!("Starting gRPC server on {}", addr);

    grpc::start_server(addr).await
}

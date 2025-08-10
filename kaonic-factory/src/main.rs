mod grpc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::SimpleLogger::new().env().init().unwrap();

    let version = env!("CARGO_PKG_VERSION");

    log::info!("Kaonic Factory Service: v{}", version);

    grpc::start_server().await?;

    Ok(())
}

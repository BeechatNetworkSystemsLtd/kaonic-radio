mod grpc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .parse_default_env()
        .init();

    let version = env!("CARGO_PKG_VERSION");

    log::info!("Kaonic Factory Service: v{}", version);

    grpc::start_server().await?;

    Ok(())
}

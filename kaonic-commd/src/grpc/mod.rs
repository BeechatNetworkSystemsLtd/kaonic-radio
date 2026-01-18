pub mod device;
pub mod network;
pub mod radio;

use std::sync::Arc;

use device::DeviceService;
use network::NetworkService;
use radio::RadioService;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

pub async fn start_server(addr: String) -> Result<(), Box<dyn std::error::Error>> {
    let addr = addr.parse()?;

    let device_service = DeviceService::default();

    // Shared cancellation token for terminating streams/tasks
    let shutdown_token = CancellationToken::new();

    let radio_controller = Arc::new(Mutex::new(
        crate::controller::RadioController::new(shutdown_token.clone()).expect("valid controller"),
    ));

    let radio_service = RadioService::new(radio_controller.clone(), shutdown_token.clone());
    let network_service = NetworkService::new(radio_controller.clone(), shutdown_token.clone());

    // Tonic server with graceful shutdown on SIGINT/SIGTERM
    let radio_controller = radio_controller.clone();

    let shutdown_signal = async move {
        // SIGTERM (Unix only)
        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
            sigterm.recv().await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        log::info!("wait shutdown listeners");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::warn!("Stopping by Ctrl+C");
            },
            _ = terminate => {
                log::warn!("Stopping by terminate");
            },
        }

        log::info!("Shutdown signal received. Cancelling tasks...");

        // Cancel token will notify all tasks
        shutdown_token.cancel();

        // Wait for controller workers to finish
        radio_controller.lock().await.wait_for_workers().await;
    };

    Server::builder()
        .add_service(kaonic::device_server::DeviceServer::new(device_service))
        .add_service(kaonic::radio_server::RadioServer::new(radio_service))
        .add_service(kaonic::network_server::NetworkServer::new(network_service))
        .serve_with_shutdown(addr, shutdown_signal)
        .await?;

    log::info!("gRPC server stopped.");

    Ok(())
}

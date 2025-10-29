pub mod device;
pub mod radio;

use device::DeviceService;
use radio::RadioService;
use tonic::transport::Server;
use tokio::sync::watch;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

pub async fn start_server(addr: String) -> Result<(), Box<dyn std::error::Error>> {
    let addr = addr.parse()?;

    let device_service = DeviceService::default();

    let mgr = crate::radio_service::RadioService::new()?;
    // Shared shutdown signal for terminating streams/tasks
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let radio_service = RadioService::new(mgr.clone(), shutdown_rx.clone());

    // Tonic server with graceful shutdown on SIGINT/SIGTERM
    let shutdown_signal = async move {
        // Ctrl+C
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };

        // SIGTERM (Unix only)
        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
            sigterm.recv().await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
        log::info!("Shutdown signal received. Stopping gRPC server...");
        // Signal receivers/streams to stop and stop radio workers
        let _ = shutdown_tx.send(true);
        mgr.shutdown();
    };

    Server::builder()
        .add_service(kaonic::device_server::DeviceServer::new(device_service))
        .add_service(kaonic::radio_server::RadioServer::new(radio_service))
        .serve_with_shutdown(addr, shutdown_signal)
        .await?;

    log::info!("gRPC server stopped.");
    Ok(())
}

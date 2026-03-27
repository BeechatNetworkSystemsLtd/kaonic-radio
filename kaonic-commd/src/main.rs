use kaonic_ctrl::{protocol::{MessageCoder, RADIO_FRAME_SIZE}, server::Server};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::grpc_server::{DeviceServer, DeviceService, GrpcRadioServer, RadioService};
use crate::radio_server::RadioServer;

mod grpc_server;
mod radio_server;

const SERVER_MTU: usize = 1400;
const SERVER_SEGMENTS: usize = 5;

const UDP_ADDR: &str = "0.0.0.0:9090";
const GRPC_ADDR: &str = "0.0.0.0:50051";

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .init();

    let version = env!("CARGO_PKG_VERSION");
    let udp_addr = UDP_ADDR.parse().expect("valid UDP listen address");
    let grpc_addr = GRPC_ADDR.parse().expect("valid gRPC listen address");

    log::info!("Kaonic Communication Daemon: v{}", version);

    let cancel = CancellationToken::new();

    let (client_send, client_recv) = mpsc::channel(16);

    let serial = read_serial();
    let radio_server = RadioServer::new(client_send, cancel.clone(), serial.clone(), RADIO_FRAME_SIZE)
        .expect("radio server");

    // Capture shared state before the UDP server takes ownership of radio_server
    let module_count = radio_server.module_count();
    let shared_radios = radio_server.radios();
    let shared_stats = radio_server.stats();
    let rx_sender = radio_server.rx_sender();

    // Start UDP server
    let server = Server::listen(
        udp_addr,
        MessageCoder::<SERVER_MTU, SERVER_SEGMENTS>::new(),
        radio_server,
        client_recv,
        cancel.clone(),
    )
    .await
    .expect("UDP server");

    // Start gRPC server sharing the same radio hardware
    let device_service = DeviceService::new(module_count, serial, RADIO_FRAME_SIZE as u32, shared_stats);
    let radio_service = RadioService::new(shared_radios, rx_sender);

    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            log::info!("gRPC server listening on {}", grpc_addr);
            if let Err(e) = tonic::transport::Server::builder()
                .add_service(DeviceServer::new(device_service))
                .add_service(GrpcRadioServer::new(radio_service))
                .serve_with_shutdown(grpc_addr, cancel.cancelled())
                .await
            {
                log::error!("gRPC server error: {}", e);
            }
        });
    }

    log::info!("server started");

    let _ = tokio::spawn(async move {
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

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::warn!("Stopping by Ctrl+C");
                cancel.cancel();
            },
            _ = terminate => {
                log::warn!("Stopping by terminate");
                cancel.cancel();
            },
        }

        log::info!("Shutdown signal received. Cancelling tasks...");
    })
    .await;


    Ok(())
}

/// Read the device serial number.
/// On Linux this comes from `/etc/machine-id`; falls back to a placeholder.
fn read_serial() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/etc/kaonic/kaonic_serial") {
            let trimmed = s.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    "unknown".to_string()
}

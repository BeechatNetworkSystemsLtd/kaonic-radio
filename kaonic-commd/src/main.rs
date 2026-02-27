use std::net::Ipv4Addr;
use std::time::Duration;

use kaonic_ctrl::{protocol::MessageCoder, server::Server};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{level_filters::LevelFilter, Level};
use tracing_subscriber::prelude::*;
use tracing_subscriber::FmtSubscriber;

use crate::radio_server::RadioServer;

mod radio_server;

const SERVER_MTU: usize = 1400;
const SERVER_SEGMENTS: usize = 5;

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .init();

    // tracing_subscriber::fmt()
    //     .with_max_level(LevelFilter::TRACE)
    //     .init();

    // let console_layer = console_subscriber::ConsoleLayer::builder()
    //     .with_default_env()
    //     .retention(Duration::from_secs(60))
    //     .publish_interval(Duration::from_millis(10))
    //     .server_addr((Ipv4Addr::UNSPECIFIED, 1234))
    //     .spawn();
    //
    // tracing_subscriber::registry()
    //     .with(console_layer)
    //     // .with(tracing_subscriber::fmt::layer())
    //     .init();

    let version = env!("CARGO_PKG_VERSION");
    let addr = "0.0.0.0:9090".parse().expect("valid listen address");

    log::info!("Kaonic Communication Daemon: v{}", version);

    let cancel = CancellationToken::new();

    let (req_send, req_recv) = broadcast::channel(16);
    let server = Server::listen(
        addr,
        req_send,
        MessageCoder::<SERVER_MTU, SERVER_SEGMENTS>::new(),
        cancel.clone(),
    )
    .await
    .expect("server");

    let radio_server = RadioServer::new(server, req_recv, cancel.clone()).expect("radio server");

    tokio::spawn(async move {
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
                cancel.cancel();
            },
            _ = terminate => {
                log::warn!("Stopping by terminate");
                cancel.cancel();
            },
        }

        log::info!("Shutdown signal received. Cancelling tasks...");
    });

    radio_server.serve().await;

    Ok(())
}

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

    let (client_send, client_recv) = mpsc::channel(16);

    let radio_server = RadioServer::new(client_send, cancel.clone()).expect("radio server");

    let server = Server::listen(
        addr,
        MessageCoder::<SERVER_MTU, SERVER_SEGMENTS>::new(),
        radio_server,
        client_recv,
        cancel.clone(),
    )
    .await
    .expect("server");

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


    log::info!("server started");

    Ok(())
}

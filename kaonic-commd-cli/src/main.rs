use std::io;
use std::time::Duration;

use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

mod app;
mod events;
mod grpc;
mod ui;

use app::App;
use grpc::GrpcEvent;

#[derive(Parser)]
#[command(name = "kaonic-commd-cli", about = "TUI for kaonic-commd gRPC interface")]
struct Args {
    /// gRPC server address
    #[arg(default_value = "http://192.168.10.1:50051")]
    server: String,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse();

    // ── Terminal setup ────────────────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, args.server).await;

    // ── Terminal teardown ─────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    server_addr: String,
) -> io::Result<()> {
    let mut app = App::new(server_addr.clone());

    // ── gRPC background task ──────────────────────────────────────────────
    let (cmd_tx, mut evt_rx) = grpc::spawn(server_addr);

    // ── Crossterm async event stream ──────────────────────────────────────
    let mut term_events = EventStream::new();

    // Ticker for forced redraws (e.g., blinking cursor)
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        // Draw
        terminal.draw(|f| ui::draw(f, &app))?;

        tokio::select! {
            // Keyboard / terminal events
            Some(Ok(evt)) = term_events.next() => {
                events::handle_event(&mut app, evt, &cmd_tx);
            }

            // gRPC events from background task
            Some(grpc_evt) = evt_rx.recv() => {
                handle_grpc_event(&mut app, grpc_evt, &cmd_tx).await;
            }

            _ = tick.tick() => {
                app.tick = app.tick.wrapping_add(1);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn handle_grpc_event(app: &mut App, evt: GrpcEvent, cmd_tx: &mpsc::Sender<grpc::GrpcCommand>) {
    match evt {
        GrpcEvent::Connected { module_count, serial, mtu, version } => {
            app.connected = true;
            app.module_count = module_count;
            app.serial = serial;
            app.mtu = mtu;
            app.version = version.clone();
            app.stats = vec![Default::default(); module_count];
            app.status_msg = format!(
                "Connected  ({} module{})",
                module_count,
                if module_count == 1 { "" } else { "s" },
            );

            // Subscribe to RX stream for each module
            for m in 0..module_count {
                let _ = cmd_tx.send(grpc::GrpcCommand::SubscribeRx { module: m as i32 }).await;
            }
        }

        GrpcEvent::Disconnected { reason } => {
            app.connected = false;
            app.status_msg = format!("Disconnected: {}", reason);
        }

        GrpcEvent::RxFrame(entry) => {
            app.push_rx(entry);
        }

        GrpcEvent::TxResult { latency_us } => {
            app.status_msg = format!("TX OK  ({} µs)", latency_us);
        }

        GrpcEvent::Statistics { module, snapshot } => {
            if module < app.stats.len() {
                app.stats[module] = snapshot;
            }
        }

        GrpcEvent::Error(msg) => {
            app.status_msg = msg;
        }
    }
}

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::app::{App, Field};
use crate::grpc::{GrpcCommand, configure_from_app};
use tokio::sync::mpsc;

/// Process one terminal event. Returns a gRPC command to send if applicable.
pub fn handle_event(app: &mut App, event: Event, cmd_tx: &mpsc::Sender<GrpcCommand>) {
    let Event::Key(key) = event else { return };
    if key.kind != KeyEventKind::Press {
        return;
    }

    // Compose window has highest priority
    if app.compose_text.is_some() {
        handle_compose_mode(app, key.code, cmd_tx);
        return;
    }

    if app.editing {
        handle_edit_mode(app, key.code, cmd_tx);
        return;
    }

    handle_normal_mode(app, key.code, key.modifiers, cmd_tx);
}

fn handle_normal_mode(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    cmd_tx: &mpsc::Sender<GrpcCommand>,
) {
    match code {
        // Quit
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
        }

        // Navigation
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => app.next_field(),
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => app.prev_field(),

        // Cycle enum fields with left/right
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => app.cycle_up(),
        KeyCode::Left | KeyCode::Char('h') => app.cycle_down(),

        // Enter edit mode for text fields, or cycle for enum fields
        KeyCode::Enter => {
            if app.is_text_field() {
                app.editing = true;
            } else {
                app.cycle_up();
            }
        }

        // Apply configuration
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(cmd) = configure_from_app(app) {
                let _ = cmd_tx.try_send(cmd);
                app.status_msg = "Configuring…".into();
            } else {
                app.status_msg = "Invalid parameters".into();
            }
        }

        // Open transmit compose window
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.compose_text = Some(String::new());
        }

        // RX log scrolling (with Shift or Ctrl)
        KeyCode::PageUp => app.scroll_rx_up(),
        KeyCode::PageDown => app.scroll_rx_down(),

        _ => {}
    }
}

fn handle_compose_mode(app: &mut App, code: KeyCode, cmd_tx: &mpsc::Sender<GrpcCommand>) {
    match code {
        KeyCode::Esc => {
            app.compose_text = None;
        }
        KeyCode::Enter => {
            if let Some(text) = app.compose_text.take() {
                if !text.is_empty() {
                    app.tx_count += 1;
                    let _ = cmd_tx.try_send(GrpcCommand::Transmit {
                        module: app.module as i32,
                        data: text.into_bytes(),
                    });
                    app.status_msg = format!("TX #{}", app.tx_count);
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut text) = app.compose_text {
                text.pop();
            }
        }
        KeyCode::Char(c) => {
            if c.is_ascii() && !c.is_ascii_control() {
                if let Some(ref mut text) = app.compose_text {
                    text.push(c);
                }
            }
        }
        _ => {}
    }
}

fn handle_edit_mode(app: &mut App, code: KeyCode, cmd_tx: &mpsc::Sender<GrpcCommand>) {
    match code {
        KeyCode::Enter => {
            app.editing = false;
            // Trigger reconnect when confirming a new server address
            if app.focused_field == Field::ServerAddr {
                let addr = app.server_addr.clone();
                app.connected = false;
                app.status_msg = "Reconnecting…".into();
                app.module_count = 0;
                let _ = cmd_tx.try_send(GrpcCommand::Reconnect { addr });
            }
        }
        KeyCode::Esc => {
            app.editing = false;
        }
        KeyCode::Backspace => app.edit_backspace(),
        KeyCode::Char(c) => {
            if app.is_numeric_field() {
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    app.edit_push(c);
                }
            } else {
                // Address field: allow any printable ASCII
                if c.is_ascii() && !c.is_ascii_control() {
                    app.edit_push(c);
                }
            }
        }
        _ => {}
    }
}

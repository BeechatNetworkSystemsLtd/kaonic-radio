use tokio::sync::mpsc;

pub mod proto {
    tonic::include_proto!("kaonic");
}

pub use proto::{
    BandwidthFilter, Empty, ModuleRequest, RadioConfig, RadioFrame, RadioModulation,
    RadioModulationFsk, RadioModulationOfdm, RadioModulationQpsk, ReceiveRequest,
    TransmitEventRequest, TransmitRequest, device_client::DeviceClient, radio_client::RadioClient,
    radio_modulation::Modulation as ProtoModulation,
};

use crate::app::{App, ModType, ModuleStatsSnapshot, RxEntry};

/// Events sent from background gRPC tasks to the TUI.
#[derive(Debug)]
pub enum GrpcEvent {
    Connected {
        module_count: usize,
        serial: String,
        mtu: u32,
        version: String,
    },
    Disconnected {
        reason: String,
    },
    RxFrame(RxEntry),
    TxFrame(RxEntry),
    TxResult {
        latency_us: u32,
    },
    Statistics {
        module: usize,
        snapshot: ModuleStatsSnapshot,
    },
    Error(String),
}

/// Commands sent from the TUI to the gRPC task.
#[derive(Debug)]
pub enum GrpcCommand {
    /// Apply both radio config and modulation in one shot (mirrors the CLI "configure" action).
    Configure {
        config: RadioConfig,
        modulation: RadioModulation,
    },
    Transmit {
        module: i32,
        data: Vec<u8>,
    },
    SubscribeRx {
        module: i32,
    },
    SubscribeTx {
        module: i32,
    },
    Reconnect {
        addr: String,
    },
}

/// Spawns the gRPC background task.  Returns (command sender, event receiver).
pub fn spawn(addr: String) -> (mpsc::Sender<GrpcCommand>, mpsc::Receiver<GrpcEvent>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<GrpcCommand>(32);
    let (evt_tx, evt_rx) = mpsc::channel::<GrpcEvent>(64);

    tokio::spawn(async move {
        let mut current_addr = addr;

        'reconnect: loop {
            // ── Connect ───────────────────────────────────────────────────
            let endpoint = match tonic::transport::Endpoint::from_shared(current_addr.clone()) {
                Ok(e) => e,
                Err(e) => {
                    let _ = evt_tx
                        .send(GrpcEvent::Disconnected {
                            reason: e.to_string(),
                        })
                        .await;
                    loop {
                        match cmd_rx.recv().await {
                            Some(GrpcCommand::Reconnect { addr }) => {
                                current_addr = addr;
                                continue 'reconnect;
                            }
                            Some(_) => {}
                            None => return,
                        }
                    }
                }
            };

            let channel = match endpoint.connect().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = evt_tx
                        .send(GrpcEvent::Disconnected {
                            reason: e.to_string(),
                        })
                        .await;
                    loop {
                        match cmd_rx.recv().await {
                            Some(GrpcCommand::Reconnect { addr }) => {
                                current_addr = addr;
                                continue 'reconnect;
                            }
                            Some(_) => {}
                            None => return,
                        }
                    }
                }
            };

            let mut device = DeviceClient::new(channel.clone());
            let mut radio = RadioClient::new(channel.clone());

            // Get device info
            match device.get_info(Empty {}).await {
                Ok(resp) => {
                    let info = resp.into_inner();
                    let _ = evt_tx
                        .send(GrpcEvent::Connected {
                            module_count: info.module_count as usize,
                            serial: info.serial,
                            mtu: info.mtu,
                            version: info.version,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = evt_tx
                        .send(GrpcEvent::Disconnected {
                            reason: e.to_string(),
                        })
                        .await;
                    loop {
                        match cmd_rx.recv().await {
                            Some(GrpcCommand::Reconnect { addr }) => {
                                current_addr = addr;
                                continue 'reconnect;
                            }
                            Some(_) => {}
                            None => return,
                        }
                    }
                }
            }

            // Spawn statistics polling task (every 1 s, all modules)
            {
                let channel2 = channel.clone();
                let evt_tx2 = evt_tx.clone();
                let module_count_for_stats = {
                    // We just stored it inside GrpcEvent::Connected; re-fetch quickly
                    let mut dc = DeviceClient::new(channel.clone());
                    dc.get_info(Empty {})
                        .await
                        .map(|r| r.into_inner().module_count as usize)
                        .unwrap_or(2)
                };

                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
                    let mut dc = DeviceClient::new(channel2);
                    loop {
                        ticker.tick().await;
                        for m in 0..module_count_for_stats {
                            match dc.get_statistics(ModuleRequest { module: m as i32 }).await {
                                Ok(resp) => {
                                    let s = resp.into_inner();
                                    let snap = ModuleStatsSnapshot {
                                        rx_packets: s.rx_packets,
                                        tx_packets: s.tx_packets,
                                        rx_bytes: s.rx_bytes,
                                        tx_bytes: s.tx_bytes,
                                        rx_errors: s.rx_errors,
                                        tx_errors: s.tx_errors,
                                    };
                                    if evt_tx2
                                        .send(GrpcEvent::Statistics {
                                            module: m,
                                            snapshot: snap,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                Err(_) => return,
                            }
                        }
                    }
                });
            }

            // ── Process commands ──────────────────────────────────────────
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    GrpcCommand::Reconnect { addr } => {
                        current_addr = addr;
                        let _ = evt_tx
                            .send(GrpcEvent::Disconnected {
                                reason: "Reconnecting…".into(),
                            })
                            .await;
                        continue 'reconnect;
                    }

                    GrpcCommand::Configure { config, modulation } => {
                        // SetConfig then SetModulation — mirrors the UDP set_config / set_modulation calls
                        match radio.set_config(config).await {
                            Err(e) => {
                                let _ = evt_tx
                                    .send(GrpcEvent::Error(format!("SetConfig: {}", e.message())))
                                    .await;
                                continue;
                            }
                            Ok(_) => {}
                        }
                        match radio.set_modulation(modulation).await {
                            Ok(_) => {
                                let _ = evt_tx.send(GrpcEvent::Error("Configure OK".into())).await;
                            }
                            Err(e) => {
                                let _ = evt_tx
                                    .send(GrpcEvent::Error(format!(
                                        "SetModulation: {}",
                                        e.message()
                                    )))
                                    .await;
                            }
                        }
                    }

                    GrpcCommand::Transmit { module, data } => {
                        let req = TransmitRequest {
                            module,
                            frame: Some(RadioFrame { data: data.into() }),
                        };
                        match radio.transmit(req).await {
                            Ok(resp) => {
                                let _ = evt_tx
                                    .send(GrpcEvent::TxResult {
                                        latency_us: resp.into_inner().latency,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = evt_tx
                                    .send(GrpcEvent::Error(format!("Transmit: {}", e.message())))
                                    .await;
                            }
                        }
                    }

                    GrpcCommand::SubscribeRx { module } => {
                        let req = ReceiveRequest { module, timeout: 0 };
                        let mut radio2 = RadioClient::new(channel.clone());
                        let evt_tx2 = evt_tx.clone();

                        tokio::spawn(async move {
                            use tokio_stream::StreamExt;

                            match radio2.receive_stream(req).await {
                                Ok(resp) => {
                                    let mut stream = resp.into_inner();
                                    while let Some(item) = stream.next().await {
                                        match item {
                                            Ok(rx) => {
                                                let bytes = rx
                                                    .frame
                                                    .map(|f| f.data.to_vec())
                                                    .unwrap_or_default();
                                                let preview = bytes
                                                    .iter()
                                                    .take(8)
                                                    .map(|b| format!("{:02X}", b))
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                let entry = RxEntry {
                                                    is_tx: false,
                                                    module: rx.module as u8,
                                                    len: bytes.len(),
                                                    rssi: Some(rx.rssi),
                                                    preview,
                                                };
                                                if evt_tx2
                                                    .send(GrpcEvent::RxFrame(entry))
                                                    .await
                                                    .is_err()
                                                {
                                                    break;
                                                }
                                            }
                                            Err(_) => break,
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = evt_tx2
                                        .send(GrpcEvent::Error(format!(
                                            "RxStream: {}",
                                            e.message()
                                        )))
                                        .await;
                                }
                            }
                        });
                    }

                    GrpcCommand::SubscribeTx { module } => {
                        let req = TransmitEventRequest { module };
                        let mut radio2 = RadioClient::new(channel.clone());
                        let evt_tx2 = evt_tx.clone();

                        tokio::spawn(async move {
                            use tokio_stream::StreamExt;

                            match radio2.transmit_event_stream(req).await {
                                Ok(resp) => {
                                    let mut stream = resp.into_inner();
                                    while let Some(item) = stream.next().await {
                                        match item {
                                            Ok(tx) => {
                                                let bytes = tx
                                                    .frame
                                                    .map(|f| f.data.to_vec())
                                                    .unwrap_or_default();
                                                let preview = bytes
                                                    .iter()
                                                    .take(8)
                                                    .map(|b| format!("{:02X}", b))
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                let entry = RxEntry {
                                                    is_tx: true,
                                                    module: tx.module as u8,
                                                    len: bytes.len(),
                                                    rssi: None,
                                                    preview,
                                                };
                                                if evt_tx2
                                                    .send(GrpcEvent::TxFrame(entry))
                                                    .await
                                                    .is_err()
                                                {
                                                    break;
                                                }
                                            }
                                            Err(_) => break,
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = evt_tx2
                                        .send(GrpcEvent::Error(format!(
                                            "TxStream: {}",
                                            e.message()
                                        )))
                                        .await;
                                }
                            }
                        });
                    }
                }
            }

            break 'reconnect;
        }
    });

    (cmd_tx, evt_rx)
}

/// Build `GrpcCommand::Configure` from current app state.
pub fn configure_from_app(app: &App) -> Option<GrpcCommand> {
    let freq: u64 = (app.freq_mhz.parse::<f64>().ok()? * 1_000_000.0) as u64;
    let channel: u32 = app.channel.parse().ok()?;
    let ch_spacing: u64 = (app.channel_spacing_khz.parse::<f64>().ok()? * 1_000.0) as u64;
    let tx_power: u32 = app.tx_power;
    let module_idx: i32 = app.module as i32;

    let config = RadioConfig {
        module: module_idx,
        freq,
        channel_spacing: ch_spacing,
        channel,
        bandwidth_filter: if app.bw_wide {
            BandwidthFilter::Wide as i32
        } else {
            BandwidthFilter::Narrow as i32
        },
    };

    let modulation_variant = match app.mod_type {
        ModType::Ofdm => Some(ProtoModulation::Ofdm(RadioModulationOfdm {
            mcs: app.ofdm_mcs.index() as u32,
            opt: app.ofdm_opt.index() as u32,
            pdt: 0x03,
            tx_power,
        })),
        ModType::Qpsk => Some(ProtoModulation::Qpsk(RadioModulationQpsk {
            chip_freq: app.qpsk_fchip.index() as u32,
            rate_mode: app.qpsk_mode.index() as u32,
            tx_power,
        })),
        ModType::Fsk => Some(ProtoModulation::Fsk(RadioModulationFsk::default())),
        ModType::Off => None,
    };

    let modulation = RadioModulation {
        module: module_idx,
        modulation: modulation_variant,
    };

    Some(GrpcCommand::Configure { config, modulation })
}

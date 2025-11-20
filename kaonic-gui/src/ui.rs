use crate::grpc_client::{GrpcClient, ReceiveEvent};
use crate::kaonic::{configuration_request::PhyConfig, QoSConfig, RadioModule, RadioPhyConfigOfdm, RadioPhyConfigQpsk};
use imgui::*;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

pub struct AppState {
    // Connection
    pub server_addr: String,
    pub connected: bool,
    pub status_message: String,

    // Radio configuration
    pub selected_module: i32,
    pub freq_mhz: f32,
    pub channel: i32,
    pub channel_spacing_khz: i32,
    pub tx_power: i32,

    // Modulation
    pub modulation_type: i32, // 0 = OFDM, 1 = QPSK
    pub ofdm_mcs: i32,
    pub ofdm_opt: i32,
    pub qpsk_chip_freq: i32,
    pub qpsk_rate_mode: i32,

    // QoS
    pub qos_enabled: bool,
    pub qos_adaptive_modulation: bool,
    pub qos_adaptive_tx_power: bool,
    pub qos_adaptive_backoff: bool,
    pub qos_cca_threshold: i32,

    // Transmit
    pub tx_data: String,
    pub tx_hex_mode: bool,
    pub last_tx_latency: Option<u32>,

    // Receive
    pub rx_events: Vec<ReceiveEvent>,
    pub rx_stream_active: bool,
    pub max_rx_events: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            server_addr: "http://192.168.0.141:8080".to_string(),
            connected: false,
            status_message: "Not connected".to_string(),

            selected_module: 0,
            freq_mhz: 915.0,
            channel: 0,
            channel_spacing_khz: 200,
            tx_power: 10,

            modulation_type: 0,
            ofdm_mcs: 3,
            ofdm_opt: 2,
            qpsk_chip_freq: 2,
            qpsk_rate_mode: 2,

            qos_enabled: false,
            qos_adaptive_modulation: true,
            qos_adaptive_tx_power: true,
            qos_adaptive_backoff: true,
            qos_cca_threshold: -75,

            tx_data: "Hello Kaonic!".to_string(),
            tx_hex_mode: false,
            last_tx_latency: None,

            rx_events: Vec::new(),
            rx_stream_active: false,
            max_rx_events: 100,
        }
    }
}

pub struct RadioGuiApp {
    client: Arc<Mutex<GrpcClient>>,
    state: Arc<Mutex<AppState>>,
    #[allow(dead_code)]
    runtime: Arc<Runtime>,
    rx_receiver: Arc<Mutex<Option<mpsc::UnboundedReceiver<ReceiveEvent>>>>,
    pub last_frame: Instant,
}

impl RadioGuiApp {
    pub fn new(
        client: Arc<Mutex<GrpcClient>>,
        state: Arc<Mutex<AppState>>,
        runtime: Arc<Runtime>,
    ) -> Self {
        Self {
            client,
            state,
            runtime,
            rx_receiver: Arc::new(Mutex::new(None)),
            last_frame: Instant::now(),
        }
    }

    pub fn render(&mut self, ui: &Ui) {
        // Process received events
        if let Some(ref mut rx) = *self.rx_receiver.lock() {
            while let Ok(event) = rx.try_recv() {
                let mut state = self.state.lock();
                state.rx_events.push(event);
                if state.rx_events.len() > state.max_rx_events {
                    state.rx_events.remove(0);
                }
            }
        }

        let display_size = ui.io().display_size;
        ui.window("Kaonic Radio Control")
            .size(display_size, Condition::Always)
            .position([0.0, 0.0], Condition::Always)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .build(|| {
                // Left column - Configuration
                ui.child_window("left_panel")
                    .size([550.0, 0.0])
                    .border(true)
                    .build(|| {
                        self.draw_connection_panel(ui);
                        ui.separator();
                        self.draw_radio_config_panel(ui);
                        ui.separator();
                        self.draw_modulation_panel(ui);
                        ui.separator();
                        self.draw_qos_panel(ui);
                        ui.separator();
                        self.draw_configure_button(ui);
                        ui.separator();
                        self.draw_transmit_panel(ui);
                    });

                ui.same_line();

                // Right column - Receive
                ui.child_window("right_panel")
                    .size([0.0, 0.0])
                    .border(true)
                    .build(|| {
                        self.draw_receive_panel(ui);
                    });
            });
    }

    fn draw_connection_panel(&mut self, ui: &Ui) {
        let mut state = self.state.lock();
        
        ui.text("Server Address:");
        ui.set_next_item_width(300.0);
        ui.input_text("##server", &mut state.server_addr).build();
        
        let addr = state.server_addr.clone();
        drop(state);

        if ui.button("Connect") {
            self.client.lock().set_server_addr(addr);

            let mut state = self.state.lock();
            match self.client.lock().get_device_info() {
                Ok(_) => {
                    state.connected = true;
                    state.status_message = "Connected successfully".to_string();
                }
                Err(e) => {
                    state.connected = false;
                    state.status_message = format!("Connection failed: {}", e);
                }
            }
        }

        ui.same_line();
        let state = self.state.lock();
        let status_color = if state.connected {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };
        ui.text_colored(status_color, &state.status_message);
    }

    fn draw_radio_config_panel(&mut self, ui: &Ui) {
        ui.text("Radio Configuration");
        ui.separator();

        let mut state = self.state.lock();

        ui.text("Module:");
        ui.same_line();
        ui.radio_button("Module A", &mut state.selected_module, 0);
        ui.same_line();
        ui.radio_button("Module B", &mut state.selected_module, 1);

        ui.text("Frequency (MHz):");
        ui.set_next_item_width(-1.0);
        Drag::new("##freq")
            .range(300.0, 2500.0)
            .speed(1.0)
            .build(ui, &mut state.freq_mhz);

        ui.text("Channel:");
        ui.set_next_item_width(-1.0);
        Drag::new("##channel")
            .range(0, 255)
            .build(ui, &mut state.channel);

        ui.text("Channel Spacing (kHz):");
        ui.set_next_item_width(-1.0);
        Drag::new("##spacing")
            .range(25, 2000)
            .speed(10.0)
            .build(ui, &mut state.channel_spacing_khz);

        ui.text("TX Power (dBm):");
        ui.set_next_item_width(-1.0);
        ui.slider("##txpower", 0, 31, &mut state.tx_power);
    }

    fn draw_modulation_panel(&mut self, ui: &Ui) {
        ui.text("Modulation");
        ui.separator();

        let mut state = self.state.lock();

        ui.text("Type:");
        ui.same_line();
        ui.radio_button("OFDM", &mut state.modulation_type, 0);
        ui.same_line();
        ui.radio_button("QPSK", &mut state.modulation_type, 1);

        if state.modulation_type == 0 {
            // OFDM
            ui.text("MCS (0=robust, 6=fast):");
            ui.set_next_item_width(-1.0);
            ui.slider("##mcs", 0, 6, &mut state.ofdm_mcs);

            ui.text("Option (interleaving):");
            ui.set_next_item_width(-1.0);
            ui.slider("##opt", 0, 3, &mut state.ofdm_opt);
        } else {
            // QPSK
            ui.text("Chip Freq (0=100k, 3=2M):");
            ui.set_next_item_width(-1.0);
            ui.slider("##chipfreq", 0, 3, &mut state.qpsk_chip_freq);

            ui.text("Rate Mode:");
            ui.set_next_item_width(-1.0);
            ui.slider("##ratemode", 0, 3, &mut state.qpsk_rate_mode);
        }
    }

    fn draw_qos_panel(&mut self, ui: &Ui) {
        ui.text("QoS Configuration");
        ui.separator();

        let mut state = self.state.lock();

        ui.checkbox("Enable QoS", &mut state.qos_enabled);

        if state.qos_enabled {
            ui.indent();
            ui.checkbox("Adaptive Modulation", &mut state.qos_adaptive_modulation);
            ui.checkbox("Adaptive TX Power", &mut state.qos_adaptive_tx_power);
            ui.checkbox("Adaptive Backoff", &mut state.qos_adaptive_backoff);

            ui.text("CCA Threshold (dBm):");
            ui.set_next_item_width(-1.0);
            ui.slider("##cca", -100, -50, &mut state.qos_cca_threshold);
            ui.unindent();
        }
    }

    fn draw_configure_button(&mut self, ui: &Ui) {
        let state = self.state.lock();
        let enabled = state.connected;
        drop(state);

        ui.enabled(enabled, || {
            if ui.button_with_size("Configure Radio", [0.0, 0.0]) {
                let state = self.state.lock();

                let module = if state.selected_module == 0 {
                    RadioModule::ModuleA
                } else {
                    RadioModule::ModuleB
                };

                let phy_config = if state.modulation_type == 0 {
                    Some(PhyConfig::Ofdm(RadioPhyConfigOfdm {
                        mcs: state.ofdm_mcs as u32,
                        opt: state.ofdm_opt as u32,
                    }))
                } else {
                    Some(PhyConfig::Qpsk(RadioPhyConfigQpsk {
                        chip_freq: state.qpsk_chip_freq as u32,
                        rate_mode: state.qpsk_rate_mode as u32,
                    }))
                };

                let qos_config = QoSConfig {
                    enabled: state.qos_enabled,
                    adaptive_modulation: state.qos_adaptive_modulation,
                    adaptive_tx_power: state.qos_adaptive_tx_power,
                    adaptive_backoff: state.qos_adaptive_backoff,
                    cca_threshold: state.qos_cca_threshold,
                };

                let result = self.client.lock().configure_radio(
                    module,
                    (state.freq_mhz * 1_000.0) as u32,
                    state.channel as u32,
                    state.channel_spacing_khz as u32,
                    state.tx_power as u32,
                    phy_config,
                    state.qos_enabled,
                    qos_config,
                );

                drop(state);
                let mut state = self.state.lock();
                match result {
                    Ok(_) => {
                        state.status_message = "Configuration applied successfully".to_string();
                    }
                    Err(e) => {
                        state.status_message = format!("Configuration failed: {}", e);
                    }
                }
            }
        });
    }

    fn draw_transmit_panel(&mut self, ui: &Ui) {
        ui.text("Transmit");
        ui.separator();

        let mut state = self.state.lock();
        let enabled = state.connected;

        ui.text("Data:");
        ui.set_next_item_width(-1.0);
        ui.input_text("##txdata", &mut state.tx_data).build();

        ui.checkbox("Hex mode", &mut state.tx_hex_mode);

        drop(state);

        ui.enabled(enabled, || {
            if ui.button_with_size("Transmit", [0.0, 0.0]) {
                let state = self.state.lock();
                let data = if state.tx_hex_mode {
                    // Parse hex string
                    let hex_str = state.tx_data.replace(" ", "").replace("0x", "");
                    (0..hex_str.len())
                        .step_by(2)
                        .filter_map(|i| {
                            let end = (i + 2).min(hex_str.len());
                            u8::from_str_radix(&hex_str[i..end], 16).ok()
                        })
                        .collect()
                } else {
                    state.tx_data.as_bytes().to_vec()
                };

                let module = if state.selected_module == 0 {
                    RadioModule::ModuleA
                } else {
                    RadioModule::ModuleB
                };
                drop(state);

                match self.client.lock().transmit_frame(module, data) {
                    Ok(latency) => {
                        let mut state = self.state.lock();
                        state.last_tx_latency = Some(latency);
                        state.status_message = format!("Transmitted successfully (latency: {} ms)", latency);
                    }
                    Err(e) => {
                        let mut state = self.state.lock();
                        state.status_message = format!("Transmit failed: {}", e);
                    }
                }
            }
        });

        let state = self.state.lock();
        if let Some(latency) = state.last_tx_latency {
            ui.text(format!("Last TX latency: {} ms", latency));
        }
    }

    fn draw_receive_panel(&mut self, ui: &Ui) {
        ui.text("Receive");
        ui.separator();

        let mut state = self.state.lock();
        let enabled = state.connected;

        ui.enabled(enabled && !state.rx_stream_active, || {
            if ui.button("Start Receiving") {
                let (tx, rx) = mpsc::unbounded_channel();
                *self.rx_receiver.lock() = Some(rx);

                let module = if state.selected_module == 0 {
                    RadioModule::ModuleA
                } else {
                    RadioModule::ModuleB
                };

                self.client.lock().start_receive_stream(module, tx);
                state.rx_stream_active = true;
                state.status_message = "Receive stream started".to_string();
            }
        });

        ui.same_line();

        ui.enabled(state.rx_stream_active, || {
            if ui.button("Stop Receiving") {
                *self.rx_receiver.lock() = None;
                state.rx_stream_active = false;
                state.status_message = "Receive stream stopped".to_string();
            }
        });

        ui.same_line();

        if ui.button("Clear") {
            state.rx_events.clear();
        }

        ui.same_line();
        ui.text(format!("Events: {}", state.rx_events.len()));

        ui.separator();

        // Display received frames
        ui.child_window("rx_events")
            .size([0.0, 0.0])
            .build(|| {
                for (idx, event) in state.rx_events.iter().rev().enumerate() {
                    ui.text(format!("#{} {}", 
                        state.rx_events.len() - idx,
                        event.timestamp.format("%H:%M:%S%.3f")
                    ));
                    ui.same_line();
                    ui.text(format!("Module: {}", if event.module == 0 { "A" } else { "B" }));
                    ui.same_line();
                    ui.text(format!("RSSI: {} dBm", event.rssi));
                    ui.same_line();
                    ui.text(format!("Latency: {} ms", event.latency));

                    let data_str = if event.frame_data.iter().all(|&b| b >= 0x20 && b <= 0x7E) {
                        format!("\"{}\"", String::from_utf8_lossy(&event.frame_data))
                    } else {
                        format!("Hex: {}", event.frame_data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "))
                    };

                    ui.text(format!("Data ({}): {}", event.frame_data.len(), data_str));
                    ui.separator();
                }
            });
    }
}

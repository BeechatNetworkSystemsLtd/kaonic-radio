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
    pub continuous_tx: bool,
    pub tx_pause_ms: i32,

    // Receive
    pub rx_events: Vec<ReceiveEvent>,
    pub rx_stream_active: bool,
    pub max_rx_events: usize,

    // RSSI visualization
    pub rssi_history: Vec<(Instant, i32)>, // (timestamp, rssi)
    pub rssi_window_secs: f32,
    
    // Waterfall data: (timestamp, rssi, payload_size)
    pub waterfall_data: Vec<(Instant, i32, usize)>,
    pub waterfall_max_entries: usize,
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
            continuous_tx: false,
            tx_pause_ms: 1000,

            rx_events: Vec::new(),
            rx_stream_active: false,
            max_rx_events: 100,

            rssi_history: Vec::new(),
            rssi_window_secs: 30.0,
            
            waterfall_data: Vec::new(),
            waterfall_max_entries: 200,
        }
    }
}

pub struct RadioGuiApp {
    client: Arc<Mutex<GrpcClient>>,
    state: Arc<Mutex<AppState>>,
    runtime: Arc<Runtime>,
    rx_receiver: Arc<Mutex<Option<mpsc::UnboundedReceiver<ReceiveEvent>>>>,
    pub last_frame: Instant,
    last_tx_time: Instant,
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
            last_tx_time: Instant::now(),
        }
    }

    pub fn render(&mut self, ui: &Ui) {
        let now = Instant::now();
        
        // Process received events
        if let Some(ref mut rx) = *self.rx_receiver.lock() {
            while let Ok(event) = rx.try_recv() {
                let mut state = self.state.lock();
                
                // Add to events list
                state.rx_events.push(event.clone());
                if state.rx_events.len() > state.max_rx_events {
                    state.rx_events.remove(0);
                }

                // Add to RSSI history with current timestamp
                state.rssi_history.push((now, event.rssi));
                
                // Add to waterfall data
                state.waterfall_data.push((now, event.rssi, event.frame_data.len()));
                if state.waterfall_data.len() > state.waterfall_max_entries {
                    state.waterfall_data.remove(0);
                }
            }
        }
        
        // Clean up old RSSI history entries (older than window)
        let mut state = self.state.lock();
        let cutoff_time = now - std::time::Duration::from_secs_f32(state.rssi_window_secs);
        state.rssi_history.retain(|(timestamp, _)| *timestamp >= cutoff_time);
        
        // Handle continuous transmission
        if state.continuous_tx && state.connected {
            let elapsed = now.duration_since(self.last_tx_time).as_millis() as i32;
            if elapsed >= state.tx_pause_ms {
                let data = if state.tx_hex_mode {
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
                
                let data_len = data.len();
                match self.client.lock().transmit_frame(module, data) {
                    Ok(latency) => {
                        let mut state = self.state.lock();
                        state.last_tx_latency = Some(latency);
                    }
                    Err(_) => {}
                }
                
                self.last_tx_time = now;
                state = self.state.lock();
            }
        }
        
        drop(state);

        let display_size = ui.io().display_size;
        ui.window("Kaonic Radio Control")
            .size(display_size, Condition::Always)
            .position([0.0, 0.0], Condition::Always)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .build(|| {
                let window_height = ui.content_region_avail()[1];
                let status_bar_height = 30.0;
                let panel_height = window_height - status_bar_height;
                
                // Left column - Configuration
                ui.child_window("left_panel")
                    .size([550.0, panel_height])
                    .border(true)
                    .build(|| {
                        // Connection Section
                        if ui.collapsing_header("Connection", TreeNodeFlags::DEFAULT_OPEN) {
                            ui.indent();
                            self.draw_connection_panel(ui);
                            ui.unindent();
                        }
                        ui.separator();
                        
                        // Configuration Section
                        if ui.collapsing_header("Configuration", TreeNodeFlags::DEFAULT_OPEN) {
                            ui.indent();
                            self.draw_radio_config_panel(ui);
                            ui.separator();
                            self.draw_modulation_panel(ui);
                            ui.separator();
                            self.draw_qos_panel(ui);
                            ui.separator();
                            self.draw_configure_button(ui);
                            ui.unindent();
                        }
                        ui.separator();
                        
                        // Transmission Section
                        if ui.collapsing_header("Transmission", TreeNodeFlags::DEFAULT_OPEN) {
                            ui.indent();
                            self.draw_transmit_panel(ui);
                            ui.unindent();
                        }
                    });

                ui.same_line();

                // Right column - Receive
                ui.child_window("right_panel")
                    .size([0.0, panel_height])
                    .border(true)
                    .build(|| {
                        // Receive Section
                        if ui.collapsing_header("Receive", TreeNodeFlags::DEFAULT_OPEN) {
                            ui.indent();
                            self.draw_receive_panel(ui);
                            ui.unindent();
                        }
                    });
                
                // Status bar at bottom
                self.draw_status_bar(ui);
            });
    }

    fn draw_connection_panel(&mut self, ui: &Ui) {
        let mut state = self.state.lock();
        
        ui.text("Server Address:");
        ui.set_next_item_width(300.0);
        ui.input_text("##server", &mut state.server_addr).build();
        
        let addr = state.server_addr.clone();
        let connected = state.connected;
        drop(state);

        let button_label = if connected { "Disconnect" } else { "Connect" };
        if ui.button(button_label) {
            if connected {
                // Disconnect
                let mut state = self.state.lock();
                state.connected = false;
                state.rx_stream_active = false;
                state.status_message = "Disconnected".to_string();
                *self.rx_receiver.lock() = None;
            } else {
                // Connect
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
        }

        
        // Auto-start receiving when connected
        let state = self.state.lock();
        let status_color = if state.connected {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };
        if state.connected && !state.rx_stream_active {
            drop(state);
            self.start_receiving();
        }
    }

    fn start_receiving(&mut self) {
        let mut state = self.state.lock();
        if !state.connected || state.rx_stream_active {
            return;
        }

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

    fn draw_radio_config_panel(&mut self, ui: &Ui) {
        ui.text("Radio");
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
            let chip_freq_label = match state.qpsk_chip_freq {
                0 => "100 kHz",
                1 => "200 kHz",
                2 => "1000 kHz",
                3 => "2000 kHz",
                _ => "Unknown",
            };
            ui.text(format!("Chip Frequency: {}", chip_freq_label));
            ui.set_next_item_width(-1.0);
            ui.slider("##chipfreq", 0, 3, &mut state.qpsk_chip_freq);

            ui.text("Rate Mode (0-3):");
            ui.set_next_item_width(-1.0);
            ui.slider("##ratemode", 0, 3, &mut state.qpsk_rate_mode);
        }
    }

    fn draw_qos_panel(&mut self, ui: &Ui) {
        ui.text("QoS");
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
                    // Convert chip_freq index to actual frequency value
                    let chip_freq_value = match state.qpsk_chip_freq {
                        0 => 100,
                        1 => 200,
                        2 => 1000,
                        3 => 2000,
                        _ => 1000, // default
                    };
                    
                    Some(PhyConfig::Qpsk(RadioPhyConfigQpsk {
                        chip_freq: chip_freq_value,
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
        let mut state = self.state.lock();
        let enabled = state.connected;

        ui.text("Data:");
        ui.set_next_item_width(-1.0);
        ui.input_text("##txdata", &mut state.tx_data).build();

        ui.checkbox("Hex mode", &mut state.tx_hex_mode);
        
        // Calculate and display data length
        let data_length = if state.tx_hex_mode {
            let hex_str = state.tx_data.replace(" ", "").replace("0x", "");
            (hex_str.len() + 1) / 2  // Number of bytes from hex string
        } else {
            state.tx_data.len()  // Number of bytes from text
        };
        ui.text(format!("Data length: {} bytes", data_length));

        ui.text("Pause between transmits (ms):");
        ui.set_next_item_width(-1.0);
        ui.slider("##txpause", 100, 5000, &mut state.tx_pause_ms);

        let continuous_enabled = state.continuous_tx;
        drop(state);

        // Single transmit button
        let _once_token = ui.begin_disabled(!enabled || continuous_enabled);
        if ui.button("Transmit Once") {
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

            let data_len = data.len();
            match self.client.lock().transmit_frame(module, data) {
                Ok(latency) => {
                    let mut state = self.state.lock();
                    state.last_tx_latency = Some(latency);
                    state.status_message = format!("Transmitted {} bytes (latency: {} ms)", data_len, latency);
                }
                Err(e) => {
                    let mut state = self.state.lock();
                    state.status_message = format!("Transmit failed: {}", e);
                }
            }
        }
        drop(_once_token);

        ui.same_line();

        // Continuous transmit control
        let _start_token = ui.begin_disabled(!enabled || continuous_enabled);
        if ui.button("Start Continuous TX") {
            let mut state = self.state.lock();
            state.continuous_tx = true;
            self.last_tx_time = Instant::now();
            state.status_message = "Continuous transmission started".to_string();
        }
        drop(_start_token);
        
        ui.same_line();
        
        let _stop_token = ui.begin_disabled(!continuous_enabled);
        if ui.button("Stop Continuous TX") {
            let mut state = self.state.lock();
            state.continuous_tx = false;
            state.status_message = "Continuous transmission stopped".to_string();
        }
        drop(_stop_token);
    }

    fn draw_receive_panel(&mut self, ui: &Ui) {
        let mut state = self.state.lock();
        let enabled = state.connected;

        // RSSI Waterfall Visualization
        ui.text("RSSI Waterfall (Vertical Scroll)");
        
        let height = 300.0;
        let max_payload_bytes = 2048.0;
        let width = max_payload_bytes; // Fixed width: 1 pixel per byte (0-2048 bytes)
        
        // Use child window with horizontal scroll for wide waterfall
        let draw_list = ui.get_window_draw_list();
        let cursor_pos = ui.cursor_screen_pos();
        
        // Constants for RSSI range
        const MIN_RSSI: i32 = -127;
        const MAX_RSSI: i32 = 10;
        const RSSI_RANGE: f32 = (MAX_RSSI - MIN_RSSI) as f32;
        
        // Background
        draw_list.add_rect(
            cursor_pos,
            [cursor_pos[0] + width, cursor_pos[1] + height],
            [0.05, 0.05, 0.1, 1.0],
        ).filled(true).build();
        
        // Helper function to convert RSSI to color (blue to red gradient)
        let rssi_to_color = |rssi: i32| -> [f32; 4] {
            let normalized = ((rssi - MIN_RSSI) as f32 / RSSI_RANGE).clamp(0.0, 1.0);
            
            // Blue -> Cyan -> Green -> Yellow -> Red gradient
            if normalized < 0.25 {
                let t = normalized / 0.25;
                [0.0, 0.0, 1.0 - t * 0.5, 1.0] // Dark blue to blue
            } else if normalized < 0.5 {
                let t = (normalized - 0.25) / 0.25;
                [0.0, t, 0.5 + t * 0.5, 1.0] // Blue to cyan
            } else if normalized < 0.75 {
                let t = (normalized - 0.5) / 0.25;
                [t, 1.0, 1.0 - t, 1.0] // Cyan to yellow
            } else {
                let t = (normalized - 0.75) / 0.25;
                [1.0, 1.0 - t, 0.0, 1.0] // Yellow to red
            }
        };
        
        // Draw waterfall: each packet as a horizontal line
        let row_height = 2.0; // Height of each packet line
        let num_visible_rows = (height / row_height) as usize;
        
        // Get current time for filtering
        let now = Instant::now();
        let time_window_secs = 30.0;
        let cutoff_time = now - std::time::Duration::from_secs_f32(time_window_secs);
        
        // Filter to only show packets from last 30 seconds
        let recent_packets: Vec<_> = state.waterfall_data.iter()
            .filter(|(timestamp, _, _)| *timestamp >= cutoff_time)
            .collect();
        
        // Draw from oldest to newest (bottom to top)
        let start_idx = if recent_packets.len() > num_visible_rows {
            recent_packets.len() - num_visible_rows
        } else {
            0
        };
        
        for (idx, (_timestamp, rssi, payload_size)) in recent_packets[start_idx..].iter().enumerate() {
            let y = cursor_pos[1] + height - ((idx + 1) as f32 * row_height);
            
            let color = rssi_to_color(*rssi);
            
            // Draw bar with width proportional to payload size (1 pixel per byte, max 2048)
            let bar_width = (*payload_size as f32).min(max_payload_bytes);
            
            draw_list.add_rect_filled_multicolor(
                [cursor_pos[0], y],
                [cursor_pos[0] + bar_width, y + row_height],
                color,
                color,
                color,
                color,
            );
        }
        
        // Draw border
        draw_list.add_rect(
            cursor_pos,
            [cursor_pos[0] + width, cursor_pos[1] + height],
            [0.4, 0.4, 0.4, 1.0],
        ).thickness(1.0).build();
        
        ui.dummy([0.0, height]);
        
        // Legend
        ui.text("Color: Blue (weak) → Cyan → Green → Yellow → Red (strong)");
        ui.text(format!("Width: Payload size (0-{} bytes) | Time window: 30 seconds", max_payload_bytes as i32));
        
        let recent_count = state.waterfall_data.iter()
            .filter(|(timestamp, _, _)| *timestamp >= now - std::time::Duration::from_secs(30))
            .count();
        ui.text(format!("Showing {} packets from last 30s | Newest at bottom", recent_count.min(num_visible_rows)));
        
        ui.separator();

        if ui.button("Clear") {
            state.rx_events.clear();
            state.rssi_history.clear();
            state.waterfall_data.clear();
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

    fn draw_status_bar(&mut self, ui: &Ui) {
        let state = self.state.lock();
        let status_color = if state.connected {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };
        
        ui.separator();
        ui.text_colored(status_color, &state.status_message);
    }
}

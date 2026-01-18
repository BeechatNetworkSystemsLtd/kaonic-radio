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

    // Bandwidth Filter
    pub bandwidth_filter: i32, // 0 = Narrow, 1 = Wide

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
    pub selected_index: Option<usize>,

    // RSSI visualization
    pub rssi_history: Vec<(Instant, i32)>, // (timestamp, rssi)
    pub rssi_window_secs: f32,
    
    // Waterfall data: (timestamp, rssi, payload_size)
    pub waterfall_data: Vec<(Instant, i32, usize)>,
    pub waterfall_max_entries: usize,
    
    // Packet type statistics
    // (packet type statistics removed)
    
    // OTA
    pub ota_file_path: String,
    pub ota_status: String,
    pub ota_version: String,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            server_addr: "192.168.10.1".to_string(),
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

            bandwidth_filter: 1, // Default to Wide

            tx_data: "Hello Kaonic!".to_string(),
            tx_hex_mode: false,
            last_tx_latency: None,
            continuous_tx: false,
            tx_pause_ms: 1000,

            rx_events: Vec::new(),
            rx_stream_active: false,
            max_rx_events: 100,
            selected_index: None,

            rssi_history: Vec::new(),
            rssi_window_secs: 30.0,
            
            waterfall_data: Vec::new(),
            waterfall_max_entries: 500,
            
            
            
            ota_file_path: String::new(),
            ota_status: String::new(),
            ota_version: String::new(),
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
                
                // (packet type statistics removed)
                
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
                        ui.separator();
                        
                        // OTA Section
                        if ui.collapsing_header("OTA Update", TreeNodeFlags::empty()) {
                            ui.indent();
                            self.draw_ota_panel(ui);
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
        
        ui.text("IP Address:");
        ui.set_next_item_width(200.0);
        ui.input_text("##server", &mut state.server_addr).build();
        
        let addr = format!("http://{}:8080", state.server_addr);
        let ip_addr = state.server_addr.clone();
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
                        drop(state);
                        
                        // Fetch firmware version
                        self.fetch_ota_version(ip_addr.clone());
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
        let should_start_rx = state.connected && !state.rx_stream_active;
        drop(state);
        
        if should_start_rx {
            self.start_receiving();
        }
    }

    pub fn start_receiving(&mut self) {
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
        
        // Color the slider orange if power > 20 dBm
        let _color_tokens = if state.tx_power > 20 {
            vec![
                ui.push_style_color(StyleColor::SliderGrab, [1.0, 0.65, 0.0, 1.0]),
                ui.push_style_color(StyleColor::SliderGrabActive, [1.0, 0.5, 0.0, 1.0]),
                ui.push_style_color(StyleColor::FrameBg, [0.3, 0.2, 0.0, 0.5]),
                ui.push_style_color(StyleColor::FrameBgHovered, [0.4, 0.25, 0.0, 0.6]),
                ui.push_style_color(StyleColor::FrameBgActive, [0.5, 0.3, 0.0, 0.7]),
            ]
        } else {
            vec![]
        };
        
        ui.slider("##txpower", 0, 31, &mut state.tx_power);
        
        for token in _color_tokens {
            token.pop();
        }

        ui.spacing();
        ui.text("Bandwidth Filter:");
        ui.same_line();
        ui.radio_button("Narrow", &mut state.bandwidth_filter, 0);
        ui.same_line();
        ui.radio_button("Wide", &mut state.bandwidth_filter, 1);
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
                    state.bandwidth_filter,
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
    
    fn upload_ota(&self, ip: String, file_path: String) {
        let state = Arc::clone(&self.state);
        
        std::thread::spawn(move || {
            let mut s = state.lock();
            s.ota_status = "Uploading...".to_string();
            drop(s);
            
            let result = (|| -> Result<String, Box<dyn std::error::Error>> {
                let url = format!("http://{}:8682/api/ota/commd/upload", ip);
                
                let file_bytes = std::fs::read(&file_path)?;
                let file_name = std::path::Path::new(&file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("firmware.zip");
                
                let part = reqwest::blocking::multipart::Part::bytes(file_bytes)
                    .file_name(file_name.to_string())
                    .mime_str("application/x-zip-compressed")?;
                
                let form = reqwest::blocking::multipart::Form::new()
                    .part("file", part);
                
                let client = reqwest::blocking::Client::new();
                let response = client.post(&url)
                    .multipart(form)
                    .send()?;
                
                let status = response.status();
                let text = response.text()?;
                
                if status.is_success() {
                    Ok(format!("OTA update successful: {}", text))
                } else {
                    Err(format!("OTA update failed ({}): {}", status, text).into())
                }
            })();
            
            let mut s = state.lock();
            s.ota_status = match result {
                Ok(msg) => msg,
                Err(e) => format!("Error: {}", e),
            };
        });
    }
    
    fn draw_ota_panel(&mut self, ui: &Ui) {
        let mut state = self.state.lock();
        let ip_addr = state.server_addr.clone();
        
        ui.set_next_item_width(250.0);
        ui.input_text("##ota_file", &mut state.ota_file_path)
            .hint("Select .zip file...")
            .build();
        
        ui.same_line();
        if ui.button("Browse...") {
            drop(state); // Release lock before blocking dialog
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("ZIP files", &["zip"])
                .pick_file()
            {
                let mut state = self.state.lock();
                state.ota_file_path = path.display().to_string();
            }
            state = self.state.lock(); // Re-acquire lock for later use
        }
        
        let ota_file = state.ota_file_path.clone();
        let ota_status = state.ota_status.clone();
        drop(state);
        
        if ui.button("Upload OTA") {
            if !ota_file.is_empty() {
                self.upload_ota(ip_addr, ota_file);
            } else {
                let mut state = self.state.lock();
                state.ota_status = "Please select a file first".to_string();
            }
        }
        
        if !ota_status.is_empty() {
            ui.text_wrapped(&ota_status);
        }
    }
    
    fn fetch_ota_version(&self, ip: String) {
        let state = Arc::clone(&self.state);
        
        std::thread::spawn(move || {
            let result = (|| -> Result<String, Box<dyn std::error::Error>> {
                let url = format!("http://{}:8682/api/ota/commd/version", ip);
                
                let client = reqwest::blocking::Client::new();
                let response = client.get(&url)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()?;
                
                if response.status().is_success() {
                    let json: serde_json::Value = response.json()?;
                    let version = json["version"].as_str().unwrap_or("unknown");
                    let hash = json["hash"].as_str().unwrap_or("");
                    
                    if version != "unknown" && !hash.is_empty() {
                        Ok(format!("{} ({})", version, &hash[..8]))
                    } else {
                        Ok("unknown".to_string())
                    }
                } else {
                    Ok("unknown".to_string())
                }
            })();
            
            let mut s = state.lock();
            s.ota_version = result.unwrap_or_else(|_| "unknown".to_string());
        });
    }

    fn draw_receive_panel(&mut self, ui: &Ui) {
        // Snapshot minimal state so we don't hold the lock across UI calls
        let total_packets = { let s = self.state.lock(); s.rx_events.len() };

        // Packet statistics
        ui.text(format!("Total: {} packets", total_packets));

        if ui.button("Clear") {
            let mut s = self.state.lock();
            s.rx_events.clear();
            s.rssi_history.clear();
            s.waterfall_data.clear();
        }

        ui.separator();

        // Snapshot events to iterate without holding the lock
        let events_snapshot = { let s = self.state.lock(); s.rx_events.clone() };

        // Determine available space and split into table + preview panels
        let avail = ui.content_region_avail();
        let table_height = (avail[1] * 0.65).max(100.0);
        let preview_height = (avail[1] - table_height).max(80.0);

        // Top: receive table
        ui.child_window("rx_events_table")
            .size([0.0, table_height])
            .border(true)
            .flags(WindowFlags::ALWAYS_VERTICAL_SCROLLBAR)
            .build(|| {
                // Table header
                ui.columns(6, "rx_table_cols", false);
                ui.text("Time"); ui.next_column();
                ui.text("Source"); ui.next_column();
                ui.text("Size"); ui.next_column();
                ui.text("RSSI"); ui.next_column();
                ui.text("Latency"); ui.next_column();
                ui.text("Preview"); ui.next_column();
                ui.separator();

                // Show rows (newest last) - we display newest at top
                for (i, event) in events_snapshot.iter().rev().enumerate() {
                    let idx = events_snapshot.len().saturating_sub(1 + i);

                    // Time column
                    let time_str = event.timestamp.format("%H:%M:%S%.3f").to_string();
                    if ui.selectable(&time_str) {
                        let mut s = self.state.lock();
                        s.selected_index = Some(idx);
                    }
                    ui.next_column();

                    // Source column (module or network)
                    let source = match event.module {
                        0 => "Module A",
                        1 => "Module B",
                        _ => "Network",
                    };
                    ui.text(source); ui.next_column();

                    // Size
                    ui.text(format!("{} B", event.frame_data.len())); ui.next_column();

                    // RSSI
                    ui.text(format!("{} dBm", event.rssi)); ui.next_column();

                    // Latency
                    ui.text(format!("{} ms", event.latency)); ui.next_column();

                    // Preview short
                    let short = event.frame_data.iter().take(8).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                    ui.text(short); ui.next_column();

                    ui.separator();
                }

                ui.columns(1, "", false);
            });

        // Bottom: preview panel (separate child window)
        ui.child_window("rx_events_preview")
            .size([0.0, preview_height])
            .border(true)
            .flags(WindowFlags::ALWAYS_VERTICAL_SCROLLBAR | WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(|| {
                ui.text("Packet Preview");
                ui.separator();

                // Clone the selected event under the lock so we can render without holding it
                let maybe_ev = {
                    let s = self.state.lock();
                    if let Some(sel) = s.selected_index {
                        if sel < s.rx_events.len() {
                            Some(s.rx_events[sel].clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(ev) = maybe_ev {
                    ui.text(format!("Time: {}", ev.timestamp.format("%Y-%m-%d %H:%M:%S%.3f")));
                    ui.text(format!("Source: {}", match ev.module {0 => "Module A", 1 => "Module B", _ => "Network"}));
                    ui.text(format!("Size: {} B", ev.frame_data.len()));
                    ui.text(format!("RSSI: {} dBm", ev.rssi));
                    ui.text(format!("Latency: {} ms", ev.latency));
                    ui.separator();
                    // Hex dump
                    let mut hex_lines: Vec<String> = Vec::new();
                    for chunk in ev.frame_data.chunks(16) {
                        let hex = chunk.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        hex_lines.push(hex);
                    }
                    for line in hex_lines {
                        ui.text(line);
                    }
                } else {
                    // Determine whether the selection is out-of-range or absent
                    let selection_state = { let s = self.state.lock(); s.selected_index };
                    if selection_state.is_some() {
                        ui.text("Selected index out of range");
                    } else {
                        ui.text("No packet selected");
                    }
                }
            });

        // (preview rendered in dedicated child window above)
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
        
        ui.same_line();
        let window_width = ui.window_size()[0];
        let mut right_pos = window_width - 20.0;
        
        // Show latest RSSI on the right with color coding
        if let Some(last_event) = state.rx_events.last() {
            let rssi_text = format!("RSSI: {} dBm", last_event.rssi);
            let rssi_width = ui.calc_text_size(&rssi_text)[0];
            right_pos -= rssi_width;
            ui.set_cursor_pos([right_pos, ui.cursor_pos()[1]]);
            
            // Color based on signal strength
            let rssi_color = if last_event.rssi >= -50 {
                [0.0, 1.0, 0.0, 1.0]  // Green: Excellent (>= -50 dBm)
            } else if last_event.rssi >= -70 {
                [0.5, 1.0, 0.0, 1.0]  // Yellow-green: Good (-50 to -70 dBm)
            } else if last_event.rssi >= -85 {
                [1.0, 0.65, 0.0, 1.0] // Orange: Fair (-70 to -85 dBm)
            } else {
                [1.0, 0.0, 0.0, 1.0]  // Red: Poor (< -85 dBm)
            };
            
            ui.text_colored(rssi_color, &rssi_text);
            ui.same_line();
        }
        
        // Show firmware version after RSSI
        if state.connected && !state.ota_version.is_empty() {
            let fw_text = format!("FW: {}", state.ota_version);
            let fw_width = ui.calc_text_size(&fw_text)[0];
            right_pos -= fw_width + 20.0;
            ui.set_cursor_pos([right_pos, ui.cursor_pos()[1]]);
            ui.text(&fw_text);
            ui.same_line();
        }
        
        // Show TX/RX statistics before RSSI
        let stats_text = format!("RX: {}", state.rx_events.len());
        let stats_width = ui.calc_text_size(&stats_text)[0];
        right_pos -= stats_width + 20.0;
        ui.set_cursor_pos([right_pos, ui.cursor_pos()[1]]);
        ui.text(&stats_text);
    }
}

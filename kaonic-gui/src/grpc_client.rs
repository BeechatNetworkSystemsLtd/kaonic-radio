use crate::kaonic::{
    device_client::DeviceClient, radio_client::RadioClient, network_client::NetworkClient,
    ConfigurationRequest, Empty, QoSConfig, RadioFrame, RadioModule, ReceiveRequest, TransmitRequest,
};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, broadcast, oneshot};
use tokio_stream::StreamExt;
use std::fmt;
use std::time::Duration;

/// Central gRPC helper that provides:
/// - a single TX queue (mpsc) serialized by a background worker
/// - an RX broadcast channel so multiple consumers can subscribe
pub struct GrpcClient {
    runtime: Arc<Runtime>,
    server_addr: Arc<StdMutex<String>>,
    tx_sender: mpsc::Sender<TxRequest>,
    rx_broadcast: broadcast::Sender<ReceiveEvent>,
}

#[derive(Clone, Debug)]
pub enum TxTarget {
    Radio(RadioModule),
    Network,
}

pub struct TxRequest {
    pub target: TxTarget,
    pub payload: Vec<u8>,
    pub resp: Option<oneshot::Sender<Result<u32, String>>>,
}

impl fmt::Debug for TxRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxRequest")
            .field("target", &"...")
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PacketType {
    Custom,
    Network,
}

#[derive(Clone, Debug)]
pub struct ReceiveEvent {
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub module: i32,
    pub frame_data: Vec<u8>,
    pub rssi: i32,
    pub latency: u32,
    pub packet_type: PacketType,
}

// Simple analyzer: check for network header
pub fn parse_network_id(data: &[u8]) -> Option<String> {
    if data.len() < kaonic_net::packet::HEADER_SIZE {
        return None;
    }
    let mut header = kaonic_net::packet::Header::new();
    match header.unpack(data) {
        Ok(_) => Some(format!("{:08X}", header.id())),
        Err(_) => None,
    }
}

/// Encode bytes into RadioFrame format
fn encode_frame(buffer: &[u8]) -> RadioFrame {
    let words = buffer
        .chunks(4)
        .map(|chunk| {
            let mut work = 0u32;
            for (i, &b) in chunk.iter().enumerate() {
                work |= (b as u32) << (i * 8);
            }
            work
        })
        .collect::<Vec<_>>();

    RadioFrame { data: words, length: buffer.len() as u32 }
}

/// Decode RadioFrame into bytes
fn decode_frame(frame: &RadioFrame) -> Vec<u8> {
    let length = frame.length as usize;
    let mut bytes = Vec::with_capacity(length);
    let mut index = 0usize;
    for word in &frame.data {
        for i in 0..4 {
            bytes.push(((word >> (i * 8)) & 0xFF) as u8);
            index += 1;
            if index >= length { break; }
        }
        if index >= length { break; }
    }
    bytes
}

impl GrpcClient {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let (tx_sender, mut tx_recv) = mpsc::channel::<TxRequest>(1024);
        let (rx_broadcast, _) = broadcast::channel::<ReceiveEvent>(1024);
        let server_addr = Arc::new(StdMutex::new("http://192.168.10.1:8080".to_string()));
        let server_addr_worker = server_addr.clone();
        let runtime_clone = runtime.clone();

        // Spawn tx worker
        runtime_clone.spawn(async move {
            while let Some(req) = tx_recv.recv().await {
                match req.target {
                    TxTarget::Network => {
                        let addr = server_addr_worker.lock().unwrap().clone();
                        let res = match NetworkClient::connect(addr).await {
                            Ok(mut client) => {
                                let frame = encode_frame(&req.payload);
                                let request = crate::kaonic::NetworkTransmitRequest { frame: Some(frame) };
                                client.transmit(tonic::Request::new(request)).await
                                    .map(|r| r.into_inner().latency)
                                    .map_err(|e| format!("Failed network transmit: {}", e))
                            }
                            Err(e) => Err(format!("Connect failed: {}", e)),
                        };
                        if let Some(resp) = req.resp { let _ = resp.send(res); }
                    }
                    TxTarget::Radio(module) => {
                        let addr = server_addr_worker.lock().unwrap().clone();
                        let res = match RadioClient::connect(addr).await {
                            Ok(mut client) => {
                                let frame = encode_frame(&req.payload);
                                let request = TransmitRequest { module: module as i32, frame: Some(frame) };
                                client.transmit(tonic::Request::new(request)).await
                                    .map(|r| r.into_inner().latency)
                                    .map_err(|e| format!("Failed radio transmit: {}", e))
                            }
                            Err(e) => Err(format!("Connect failed: {}", e)),
                        };
                        if let Some(resp) = req.resp { let _ = resp.send(res); }
                    }
                }
            }
        });

        Self { runtime, server_addr, tx_sender, rx_broadcast }
    }

    /// Subscribe to receive broadcast
    pub fn rx_subscribe(&self) -> broadcast::Receiver<ReceiveEvent> {
        self.rx_broadcast.subscribe()
    }

    /// Enqueue tx request
    pub fn tx_enqueue(&self, req: TxRequest) -> Result<(), String> {
        self.tx_sender.try_send(req).map_err(|e| format!("TX queue full: {}", e))
    }

    /// Blocking send convenience (uses oneshot response and blocks current thread by using runtime.block_on)
    pub fn tx_send_blocking(&self, target: TxTarget, payload: Vec<u8>, timeout_ms: Option<u64>) -> Result<u32, String> {
        let (tx, rx) = oneshot::channel::<Result<u32, String>>();
        let req = TxRequest { target, payload, resp: Some(tx) };
        self.tx_enqueue(req)?;
        if let Some(ms) = timeout_ms {
            let dur = Duration::from_millis(ms);
            return self.runtime.block_on(async move {
                match tokio::time::timeout(dur, rx).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(_)) => Err("TX worker dropped response".to_string()),
                    Err(_) => Err("TX response timeout".to_string()),
                }
            });
        }
        self.runtime.block_on(async move { rx.await.map_err(|_| "TX worker dropped response".to_string())? })
    }

    pub fn set_server_addr(&mut self, addr: String) {
        if let Ok(mut s) = self.server_addr.lock() {
            *s = addr;
        }
    }

    pub fn get_server_addr(&self) -> String { self.server_addr.lock().unwrap().clone() }

    pub async fn connect_device(&self) -> Result<DeviceClient<tonic::transport::Channel>, String> {
        DeviceClient::connect(self.server_addr.lock().unwrap().clone())
            .await
            .map_err(|e| format!("Failed to connect: {}", e))
    }

    pub async fn connect_radio(&self) -> Result<RadioClient<tonic::transport::Channel>, String> {
        RadioClient::connect(self.server_addr.lock().unwrap().clone())
            .await
            .map_err(|e| format!("Failed to connect: {}", e))
    }

    pub fn get_device_info(&self) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut client = self.connect_device().await?;
            client.get_info(tonic::Request::new(Empty {})).await.map_err(|e| format!("Failed to get device info: {}", e))?;
            Ok(())
        })
    }

    pub fn configure_radio(&self, module: RadioModule, freq: u32, channel: u32, channel_spacing: u32, tx_power: u32, phy_config: Option<crate::kaonic::configuration_request::PhyConfig>, qos_enabled: bool, qos_config: QoSConfig, bandwidth_filter: i32) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut client = self.connect_radio().await?;
            let qos = Some(QoSConfig {
                enabled: qos_enabled,
                adaptive_modulation: qos_config.adaptive_modulation,
                adaptive_tx_power: qos_config.adaptive_tx_power,
                adaptive_backoff: qos_config.adaptive_backoff,
                cca_threshold: qos_config.cca_threshold,
            });
            let request = ConfigurationRequest { module: module as i32, freq, channel, channel_spacing, tx_power, phy_config, qos, bandwidth_filter };
            client.configure(tonic::Request::new(request)).await.map_err(|e| format!("Failed to configure: {}", e))?;
            Ok(())
        })
    }

    /// Convenience direct transmit (kept for backward compatibility)
    #[allow(dead_code)]
    pub fn transmit_frame(&self, module: RadioModule, data: Vec<u8>) -> Result<u32, String> {
        self.runtime.block_on(async {
            let mut client = self.connect_radio().await?;
            let frame = encode_frame(&data);
            let request = TransmitRequest { module: module as i32, frame: Some(frame) };
            let response = client.transmit(tonic::Request::new(request)).await.map_err(|e| format!("Failed to transmit: {}", e))?;
            Ok(response.into_inner().latency)
        })
    }

    /// Convenience direct network transmit (compat)
    #[allow(dead_code)]
    pub fn network_transmit(&self, data: Vec<u8>) -> Result<u32, String> {
        self.runtime.block_on(async {
            let mut client = NetworkClient::connect(self.server_addr.lock().unwrap().clone()).await.map_err(|e| format!("Failed to connect: {}", e))?;
            let frame = encode_frame(&data);
            let request = crate::kaonic::NetworkTransmitRequest { frame: Some(frame) };
            let response = client.transmit(tonic::Request::new(request)).await.map_err(|e| format!("Failed to transmit network frame: {}", e))?;
            Ok(response.into_inner().latency)
        })
    }

    /// Start a receive stream for a radio module; published events are sent to the provided `rx` and
    /// also published on the broadcast channel for additional subscribers.
    pub fn start_receive_stream(&self, module: RadioModule, rx: mpsc::UnboundedSender<ReceiveEvent>) {
        let server_addr = self.server_addr.clone();
        let rx_broadcast = self.rx_broadcast.clone();
        let runtime = self.runtime.clone();

        runtime.spawn(async move {
            loop {
                let addr = server_addr.lock().unwrap().clone();
                match RadioClient::connect(addr).await {
                    Ok(mut client) => {
                        let request = ReceiveRequest { module: module as i32, timeout: 1000 };
                        match client.receive_stream(tonic::Request::new(request)).await {
                            Ok(response) => {
                                let mut stream = response.into_inner();
                                while let Some(result) = stream.next().await {
                                    match result {
                                        Ok(rx_response) => {
                                            let frame_data = if let Some(frame) = rx_response.frame { decode_frame(&frame) } else { Vec::new() };
                                            let packet_type = if frame_data.len() >= kaonic_net::packet::HEADER_SIZE { PacketType::Network } else { PacketType::Custom };
                                            let event = ReceiveEvent { timestamp: chrono::Local::now(), module: rx_response.module, frame_data: frame_data.clone(), rssi: rx_response.rssi, latency: rx_response.latency, packet_type };
                                            // deliver to provided receiver
                                            if rx.send(event.clone()).is_err() { return; }
                                            // publish to broadcast
                                            let _ = rx_broadcast.send(event);
                                        }
                                        Err(e) => { eprintln!("Stream error: {}", e); break; }
                                    }
                                }
                            }
                            Err(e) => { eprintln!("Failed to start receive stream: {}", e); }
                        }
                    }
                    Err(e) => { eprintln!("Failed to connect: {}", e); }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });
    }

    /// Start network receive stream
    pub fn start_network_receive_stream(&self, rx: mpsc::UnboundedSender<ReceiveEvent>) {
        let server_addr = self.server_addr.clone();
        let rx_broadcast = self.rx_broadcast.clone();
        let runtime = self.runtime.clone();

        runtime.spawn(async move {
            loop {
                let addr = server_addr.lock().unwrap().clone();
                match NetworkClient::connect(addr).await {
                    Ok(mut client) => {
                        let request = crate::kaonic::NetworkReceiveRequest {};
                        match client.receive_stream(tonic::Request::new(request)).await {
                            Ok(response) => {
                                let mut stream = response.into_inner();
                                while let Some(result) = stream.next().await {
                                    match result {
                                        Ok(rx_response) => {
                                            let frame_data = if let Some(frame) = rx_response.frame { decode_frame(&frame) } else { Vec::new() };
                                            let event = ReceiveEvent { timestamp: chrono::Local::now(), module: -1, frame_data: frame_data.clone(), rssi: rx_response.rssi, latency: rx_response.latency, packet_type: PacketType::Network };
                                            if rx.send(event.clone()).is_err() { return; }
                                            let _ = rx_broadcast.send(event);
                                        }
                                        Err(e) => { eprintln!("Network stream error: {}", e); break; }
                                    }
                                }
                            }
                            Err(e) => { eprintln!("Failed to start network receive stream: {}", e); }
                        }
                    }
                    Err(e) => { eprintln!("Failed to connect network client: {}", e); }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });
    }
}

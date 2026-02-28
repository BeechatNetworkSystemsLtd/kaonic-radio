use kaonic_ctrl::{client::Client, protocol::MessageCoder, radio::RadioClient};
use kaonic_frame::frame::Frame;
use radio_common::{
    frequency::BandwidthFilter,
    modulation::{OfdmBandwidthOption, OfdmMcs, OfdmModulation, QpskChipFrequency, QpskModulation, QpskRateMode},
    Hertz, Modulation, RadioConfig,
};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;
use std::fmt;
use std::time::Duration;

/// Module selector (mirrors the old gRPC RadioModule for API compatibility)
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(usize)]
pub enum RadioModule {
    ModuleA = 0,
    ModuleB = 1,
}

/// PHY configuration (mirrors old gRPC types for API compatibility)
pub struct RadioPhyConfigOfdm {
    pub mcs: u32,
    pub opt: u32,
}

pub struct RadioPhyConfigQpsk {
    pub chip_freq: u32,
    pub rate_mode: u32,
}

pub enum PhyConfig {
    Ofdm(RadioPhyConfigOfdm),
    Qpsk(RadioPhyConfigQpsk),
}

/// QoS configuration (kept for API compatibility; not applied via binary protocol)
pub struct QoSConfig {
    pub enabled: bool,
    pub adaptive_modulation: bool,
    pub adaptive_tx_power: bool,
    pub adaptive_backoff: bool,
    pub cca_threshold: i32,
}

/// Central client that provides a TX queue and RX broadcast channel backed
/// by the kaonic-ctrl binary protocol (UDP) instead of gRPC.
pub struct GrpcClient {
    runtime: Arc<Runtime>,
    server_addr: Arc<StdMutex<String>>,
    tx_sender: mpsc::Sender<TxRequest>,
    rx_broadcast: broadcast::Sender<ReceiveEvent>,
    radio_client: Arc<AsyncMutex<Option<RadioClient>>>,
    rx_started: Arc<StdMutex<bool>>,
}

#[derive(Clone, Copy, Debug)]
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

/// Check whether data begins with a kaonic-net network packet header.
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

impl GrpcClient {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let (tx_sender, mut tx_recv) = mpsc::channel::<TxRequest>(1024);
        let (rx_broadcast, _) = broadcast::channel::<ReceiveEvent>(1024);
        let server_addr = Arc::new(StdMutex::new("192.168.10.1:9090".to_string()));
        let radio_client: Arc<AsyncMutex<Option<RadioClient>>> =
            Arc::new(AsyncMutex::new(None));
        let rx_started = Arc::new(StdMutex::new(false));

        let radio_client_worker = radio_client.clone();
        let runtime_clone = runtime.clone();

        // Background TX worker: dequeues requests and transmits via RadioClient
        runtime_clone.spawn(async move {
            while let Some(req) = tx_recv.recv().await {
                let module_idx = match req.target {
                    TxTarget::Radio(m) => m as usize,
                    TxTarget::Network => 0,
                };
                let mut rc = radio_client_worker.lock().await;
                let res = if let Some(ref mut client) = *rc {
                    let mut frame = Frame::<2048>::new();
                    frame.copy_from_slice(&req.payload);
                    client
                        .transmit(module_idx, &frame)
                        .await
                        .map(|_| 0u32)
                        .map_err(|e| format!("TX error: {:?}", e))
                } else {
                    Err("Not connected".to_string())
                };
                if let Some(resp) = req.resp {
                    let _ = resp.send(res);
                }
            }
        });

        Self {
            runtime,
            server_addr,
            tx_sender,
            rx_broadcast,
            radio_client,
            rx_started,
        }
    }

    /// Subscribe to the receive broadcast channel.
    pub fn rx_subscribe(&self) -> broadcast::Receiver<ReceiveEvent> {
        self.rx_broadcast.subscribe()
    }

    /// Non-blocking enqueue of a TX request.
    pub fn tx_enqueue(&self, req: TxRequest) -> Result<(), String> {
        self.tx_sender
            .try_send(req)
            .map_err(|e| format!("TX queue full: {}", e))
    }

    /// Blocking transmit with optional timeout (ms).  Returns 0 for latency
    /// since the binary protocol does not report it.
    pub fn tx_send_blocking(
        &self,
        target: TxTarget,
        payload: Vec<u8>,
        timeout_ms: Option<u64>,
    ) -> Result<u32, String> {
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
        self.runtime.block_on(async move {
            rx.await
                .map_err(|_| "TX worker dropped response".to_string())?
        })
    }

    pub fn set_server_addr(&mut self, addr: String) {
        // Cancel the current connection so that the next get_device_info() starts fresh.
        self.runtime.block_on(async {
            let mut rc = self.radio_client.lock().await;
            if let Some(ref mut client) = *rc {
                client.cancel();
            }
            *rc = None;
        });
        *self.rx_started.lock().unwrap() = false;
        if let Ok(mut s) = self.server_addr.lock() {
            *s = addr;
        }
    }

    pub fn get_server_addr(&self) -> String {
        self.server_addr.lock().unwrap().clone()
    }

    /// Connect to kaonic-commd via UDP, verify with a GetInfo round-trip, and
    /// store the RadioClient for subsequent operations.
    pub fn get_device_info(&self) -> Result<(), String> {
        let addr_str = self.server_addr.lock().unwrap().clone();
        let server_addr: std::net::SocketAddr = addr_str
            .parse()
            .map_err(|e| format!("Invalid address '{}': {}", addr_str, e))?;
        let listen_addr: std::net::SocketAddr = "0.0.0.0:0".parse().unwrap();
        let radio_client = self.radio_client.clone();
        let rx_started = self.rx_started.clone();

        self.runtime.block_on(async move {
            let cancel = CancellationToken::new();
            let client = Client::connect(
                listen_addr,
                server_addr,
                MessageCoder::<1400, 5>::new(),
                cancel.clone(),
            )
            .await
            .map_err(|e| format!("Connect error: {:?}", e))?;

            let mut rc = RadioClient::new(client, cancel)
                .await
                .map_err(|e| format!("RadioClient error: {:?}", e))?;

            rc.get_info()
                .await
                .map_err(|e| format!("GetInfo error: {:?}", e))?;

            *rx_started.lock().unwrap() = false;
            *radio_client.lock().await = Some(rc);
            Ok(())
        })
    }

    /// Apply radio frequency/channel configuration and modulation.
    /// QoS parameters are accepted for API compatibility but are not forwarded
    /// (the binary protocol does not support them).
    pub fn configure_radio(
        &self,
        module: RadioModule,
        freq: u32,
        channel: u32,
        channel_spacing: u32,
        tx_power: u32,
        phy_config: Option<PhyConfig>,
        _qos_enabled: bool,
        _qos_config: QoSConfig,
        bandwidth_filter: i32,
    ) -> Result<(), String> {
        let module_idx = module as usize;
        let bw = if bandwidth_filter == 0 {
            BandwidthFilter::Narrow
        } else {
            BandwidthFilter::Wide
        };
        let config = RadioConfig {
            freq: Hertz::from_khz(freq as u64),
            channel: channel as u16,
            channel_spacing: Hertz::from_khz(channel_spacing as u64),
            bandwidth_filter: bw,
        };
        let modulation = phy_config.map(|pc| match pc {
            PhyConfig::Ofdm(ofdm) => {
                let mcs = match ofdm.mcs {
                    0 => OfdmMcs::BpskC1_2_4x,
                    1 => OfdmMcs::BpskC1_2_2x,
                    2 => OfdmMcs::QpskC1_2_2x,
                    3 => OfdmMcs::QpskC1_2,
                    4 => OfdmMcs::QpskC3_4,
                    5 => OfdmMcs::QamC1_2,
                    _ => OfdmMcs::QamC3_4,
                };
                let opt = match ofdm.opt {
                    0 => OfdmBandwidthOption::Option1,
                    1 => OfdmBandwidthOption::Option2,
                    2 => OfdmBandwidthOption::Option3,
                    _ => OfdmBandwidthOption::Option4,
                };
                Modulation::Ofdm(OfdmModulation {
                    mcs,
                    opt,
                    pdt: 0x03,
                    tx_power: tx_power as u8,
                })
            }
            PhyConfig::Qpsk(qpsk) => {
                let fchip = match qpsk.chip_freq {
                    100 => QpskChipFrequency::Fchip100,
                    200 => QpskChipFrequency::Fchip200,
                    1000 => QpskChipFrequency::Fchip1000,
                    _ => QpskChipFrequency::Fchip2000,
                };
                let mode = match qpsk.rate_mode {
                    0 => QpskRateMode::RateMode0,
                    1 => QpskRateMode::RateMode1,
                    2 => QpskRateMode::RateMode2,
                    3 => QpskRateMode::RateMode3,
                    _ => QpskRateMode::RateMode4,
                };
                Modulation::Qpsk(QpskModulation {
                    fchip,
                    mode,
                    tx_power: tx_power as u8,
                })
            }
        });

        let radio_client = self.radio_client.clone();
        self.runtime.block_on(async move {
            let mut rc = radio_client.lock().await;
            if let Some(ref mut client) = *rc {
                client
                    .set_radio_config(module_idx, config)
                    .await
                    .map_err(|e| format!("Config error: {:?}", e))?;
                if let Some(mod_val) = modulation {
                    client
                        .set_modulation(module_idx, mod_val)
                        .await
                        .map_err(|e| format!("Modulation error: {:?}", e))?;
                }
                Ok(())
            } else {
                Err("Not connected".to_string())
            }
        })
    }

    /// Start a background task that forwards all received radio frames to `rx`
    /// and to the broadcast channel.  Only the first call per connection spawns
    /// a listener; subsequent calls (e.g. for a second module) are no-ops
    /// because module_receive() already delivers frames for every module.
    pub fn start_receive_stream(
        &self,
        _module: RadioModule,
        rx: mpsc::UnboundedSender<ReceiveEvent>,
    ) {
        {
            let mut started = self.rx_started.lock().unwrap();
            if *started {
                return;
            }
            *started = true;
        }

        let radio_client = self.radio_client.clone();
        let rx_broadcast = self.rx_broadcast.clone();

        self.runtime.spawn(async move {
            let mut module_rx = {
                let rc = radio_client.lock().await;
                match *rc {
                    Some(ref client) => client.module_receive(),
                    None => return,
                }
            };

            loop {
                match module_rx.recv().await {
                    Ok(rx_module) => {
                        let frame_data = rx_module.frame.as_slice().to_vec();
                        let packet_type =
                            if frame_data.len() >= kaonic_net::packet::HEADER_SIZE {
                                PacketType::Network
                            } else {
                                PacketType::Custom
                            };
                        let event = ReceiveEvent {
                            timestamp: chrono::Local::now(),
                            module: rx_module.module as i32,
                            frame_data,
                            rssi: 0,
                            latency: 0,
                            packet_type,
                        };
                        if rx.send(event.clone()).is_err() {
                            return;
                        }
                        let _ = rx_broadcast.send(event);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    /// No-op: radio frames already include network-layer packets; no separate
    /// network receive stream is needed with the binary protocol.
    pub fn start_network_receive_stream(&self, _rx: mpsc::UnboundedSender<ReceiveEvent>) {}
}


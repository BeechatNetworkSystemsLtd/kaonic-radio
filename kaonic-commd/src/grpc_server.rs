use std::time::Instant;

use kaonic_ctrl::protocol::{ReceiveModule, TransmitModule};
use kaonic_radio::{platform::PlatformRadioFrame, radio::Radio};
use radio_common::{
    RadioConfig,
    frequency::{BandwidthFilter, Hertz},
    modulation::{
        Modulation, OfdmBandwidthOption, OfdmMcs, OfdmModulation, QpskChipFrequency,
        QpskModulation, QpskRateMode,
    },
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::radio_server::{SharedModuleStats, SharedRadio};

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

pub use kaonic::device_server::DeviceServer;
pub use kaonic::radio_server::RadioServer as GrpcRadioServer;

use kaonic::{
    Empty, InfoResponse, ModuleRequest, RadioConfig as ProtoRadioConfig, RadioFrame as ProtoFrame,
    RadioModulation, RadioModulationFsk, RadioModulationOfdm, RadioModulationQpsk, ReceiveRequest,
    ReceiveResponse, StatisticsResponse, TransmitEventRequest, TransmitEventResponse,
    TransmitRequest, TransmitResponse, device_server::Device,
    radio_modulation::Modulation as ProtoModulation, radio_server::Radio as RadioTrait,
};

//***********************************************************************************************//
// Helpers — RadioFrame
//***********************************************************************************************//

fn frame_to_bytes(frame: &ProtoFrame) -> Vec<u8> {
    frame.data.to_vec()
}

fn bytes_to_frame(data: &[u8]) -> ProtoFrame {
    ProtoFrame {
        data: data.to_vec().into(),
    }
}

//***********************************************************************************************//
// Helpers — enum conversions (proto ↔ radio-common)
//***********************************************************************************************//

fn ofdm_mcs_from_u32(v: u32) -> OfdmMcs {
    match v {
        0 => OfdmMcs::BpskC1_2_4x,
        1 => OfdmMcs::BpskC1_2_2x,
        2 => OfdmMcs::QpskC1_2_2x,
        3 => OfdmMcs::QpskC1_2,
        4 => OfdmMcs::QpskC3_4,
        5 => OfdmMcs::QamC1_2,
        6 => OfdmMcs::QamC3_4,
        _ => OfdmMcs::QamC3_4,
    }
}

fn ofdm_mcs_to_u32(mcs: &OfdmMcs) -> u32 {
    match mcs {
        OfdmMcs::BpskC1_2_4x => 0,
        OfdmMcs::BpskC1_2_2x => 1,
        OfdmMcs::QpskC1_2_2x => 2,
        OfdmMcs::QpskC1_2 => 3,
        OfdmMcs::QpskC3_4 => 4,
        OfdmMcs::QamC1_2 => 5,
        OfdmMcs::QamC3_4 => 6,
    }
}

fn ofdm_opt_from_u32(v: u32) -> OfdmBandwidthOption {
    match v {
        0 => OfdmBandwidthOption::Option1,
        1 => OfdmBandwidthOption::Option2,
        2 => OfdmBandwidthOption::Option3,
        3 => OfdmBandwidthOption::Option4,
        _ => OfdmBandwidthOption::Option1,
    }
}

fn ofdm_opt_to_u32(opt: &OfdmBandwidthOption) -> u32 {
    match opt {
        OfdmBandwidthOption::Option1 => 0,
        OfdmBandwidthOption::Option2 => 1,
        OfdmBandwidthOption::Option3 => 2,
        OfdmBandwidthOption::Option4 => 3,
    }
}

fn qpsk_fchip_from_u32(v: u32) -> QpskChipFrequency {
    match v {
        0 => QpskChipFrequency::Fchip100,
        1 => QpskChipFrequency::Fchip200,
        2 => QpskChipFrequency::Fchip1000,
        3 => QpskChipFrequency::Fchip2000,
        _ => QpskChipFrequency::Fchip100,
    }
}

fn qpsk_fchip_to_u32(fchip: &QpskChipFrequency) -> u32 {
    match fchip {
        QpskChipFrequency::Fchip100 => 0,
        QpskChipFrequency::Fchip200 => 1,
        QpskChipFrequency::Fchip1000 => 2,
        QpskChipFrequency::Fchip2000 => 3,
    }
}

fn qpsk_mode_from_u32(v: u32) -> QpskRateMode {
    match v {
        0 => QpskRateMode::RateMode0,
        1 => QpskRateMode::RateMode1,
        2 => QpskRateMode::RateMode2,
        3 => QpskRateMode::RateMode3,
        4 => QpskRateMode::RateMode4,
        _ => QpskRateMode::RateMode0,
    }
}

fn qpsk_mode_to_u32(mode: &QpskRateMode) -> u32 {
    match mode {
        QpskRateMode::RateMode0 => 0,
        QpskRateMode::RateMode1 => 1,
        QpskRateMode::RateMode2 => 2,
        QpskRateMode::RateMode3 => 3,
        QpskRateMode::RateMode4 => 4,
    }
}

fn modulation_to_proto(module: i32, modulation: &Modulation) -> RadioModulation {
    let variant = match modulation {
        Modulation::Ofdm(o) => Some(ProtoModulation::Ofdm(RadioModulationOfdm {
            mcs: ofdm_mcs_to_u32(&o.mcs),
            opt: ofdm_opt_to_u32(&o.opt),
            pdt: o.pdt as u32,
            tx_power: o.tx_power as u32,
        })),
        Modulation::Qpsk(q) => Some(ProtoModulation::Qpsk(RadioModulationQpsk {
            chip_freq: qpsk_fchip_to_u32(&q.fchip),
            rate_mode: qpsk_mode_to_u32(&q.mode),
            tx_power: q.tx_power as u32,
        })),
        Modulation::Fsk => Some(ProtoModulation::Fsk(RadioModulationFsk::default())),
        Modulation::Off => None,
    };
    RadioModulation {
        module,
        modulation: variant,
    }
}

fn modulation_from_proto(req: &RadioModulation) -> Modulation {
    match &req.modulation {
        Some(ProtoModulation::Ofdm(o)) => Modulation::Ofdm(OfdmModulation {
            mcs: ofdm_mcs_from_u32(o.mcs),
            opt: ofdm_opt_from_u32(o.opt),
            pdt: o.pdt as u8,
            tx_power: o.tx_power as u8,
        }),
        Some(ProtoModulation::Qpsk(q)) => Modulation::Qpsk(QpskModulation {
            fchip: qpsk_fchip_from_u32(q.chip_freq),
            mode: qpsk_mode_from_u32(q.rate_mode),
            tx_power: q.tx_power as u8,
        }),
        Some(ProtoModulation::Fsk(_)) => Modulation::Fsk,
        None => Modulation::Off,
    }
}

fn config_to_proto(module: i32, cfg: &RadioConfig) -> ProtoRadioConfig {
    ProtoRadioConfig {
        module,
        freq: cfg.freq.as_hz(),
        channel_spacing: cfg.channel_spacing.as_hz(),
        channel: cfg.channel as u32,
        bandwidth_filter: match cfg.bandwidth_filter {
            BandwidthFilter::Wide => 1,
            BandwidthFilter::Narrow => 0,
        },
    }
}

fn config_from_proto(req: &ProtoRadioConfig) -> RadioConfig {
    RadioConfig {
        freq: Hertz::new(req.freq),
        channel_spacing: Hertz::new(req.channel_spacing),
        channel: req.channel as u16,
        bandwidth_filter: match req.bandwidth_filter {
            1 => BandwidthFilter::Wide,
            _ => BandwidthFilter::Narrow,
        },
    }
}

//***********************************************************************************************//
// Device service
//***********************************************************************************************//

pub struct DeviceService {
    module_count: usize,
    serial: String,
    mtu: u32,
    version: &'static str,
    stats: Vec<SharedModuleStats>,
}

impl DeviceService {
    pub fn new(
        module_count: usize,
        serial: String,
        mtu: u32,
        stats: Vec<SharedModuleStats>,
    ) -> Self {
        Self {
            module_count,
            serial,
            mtu,
            version: env!("CARGO_PKG_VERSION"),
            stats,
        }
    }
}

#[tonic::async_trait]
impl Device for DeviceService {
    async fn get_info(&self, _: Request<Empty>) -> Result<Response<InfoResponse>, Status> {
        Ok(Response::new(InfoResponse {
            module_count: self.module_count as u32,
            serial: self.serial.clone(),
            mtu: self.mtu,
            version: self.version.to_string(),
        }))
    }

    async fn get_statistics(
        &self,
        request: Request<ModuleRequest>,
    ) -> Result<Response<StatisticsResponse>, Status> {
        use std::sync::atomic::Ordering;
        let idx = request.into_inner().module as usize;
        if idx >= self.stats.len() {
            return Err(Status::invalid_argument(format!(
                "module {} out of range",
                idx
            )));
        }
        let s = &self.stats[idx];
        Ok(Response::new(StatisticsResponse {
            rx_packets: s.rx_packets.load(Ordering::Relaxed),
            tx_packets: s.tx_packets.load(Ordering::Relaxed),
            rx_bytes: s.rx_bytes.load(Ordering::Relaxed),
            tx_bytes: s.tx_bytes.load(Ordering::Relaxed),
            rx_errors: s.rx_errors.load(Ordering::Relaxed),
            tx_errors: s.tx_errors.load(Ordering::Relaxed),
        }))
    }
}

//***********************************************************************************************//
// Radio service
//***********************************************************************************************//

pub struct RadioService {
    radios: Vec<SharedRadio>,
    module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
    module_tx_send: broadcast::Sender<Box<TransmitModule>>,
}

impl RadioService {
    pub fn new(
        radios: Vec<SharedRadio>,
        module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
        module_tx_send: broadcast::Sender<Box<TransmitModule>>,
    ) -> Self {
        Self {
            radios,
            module_rx_send,
            module_tx_send,
        }
    }

    fn module_index(&self, module: i32) -> Result<usize, Status> {
        if module < 0 || module as usize >= self.radios.len() {
            return Err(Status::invalid_argument(format!(
                "module {} out of range (have {})",
                module,
                self.radios.len()
            )));
        }
        Ok(module as usize)
    }
}

#[tonic::async_trait]
impl RadioTrait for RadioService {
    // ── GetConfig ───────────────────────────────────────────────────────────

    async fn get_config(
        &self,
        request: Request<ModuleRequest>,
    ) -> Result<Response<ProtoRadioConfig>, Status> {
        let module = request.into_inner().module;
        let idx = self.module_index(module)?;
        let cfg = self.radios[idx].lock().unwrap().get_config();
        Ok(Response::new(config_to_proto(module, &cfg)))
    }

    // ── SetConfig ───────────────────────────────────────────────────────────

    async fn set_config(
        &self,
        request: Request<ProtoRadioConfig>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let idx = self.module_index(req.module)?;
        let cfg = config_from_proto(&req);
        self.radios[idx]
            .lock()
            .unwrap()
            .set_config(&cfg)
            .map_err(|e| Status::internal(format!("set_config: {:?}", e)))?;
        Ok(Response::new(Empty {}))
    }

    // ── GetModulation ───────────────────────────────────────────────────────

    async fn get_modulation(
        &self,
        request: Request<ModuleRequest>,
    ) -> Result<Response<RadioModulation>, Status> {
        let module = request.into_inner().module;
        let idx = self.module_index(module)?;
        let modulation = self.radios[idx].lock().unwrap().get_modulation();
        Ok(Response::new(modulation_to_proto(module, &modulation)))
    }

    // ── SetModulation ───────────────────────────────────────────────────────

    async fn set_modulation(
        &self,
        request: Request<RadioModulation>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let idx = self.module_index(req.module)?;
        let modulation = modulation_from_proto(&req);
        self.radios[idx]
            .lock()
            .unwrap()
            .set_modulation(&modulation)
            .map_err(|e| Status::internal(format!("set_modulation: {:?}", e)))?;
        Ok(Response::new(Empty {}))
    }

    // ── Transmit ────────────────────────────────────────────────────────────

    async fn transmit(
        &self,
        request: Request<TransmitRequest>,
    ) -> Result<Response<TransmitResponse>, Status> {
        let req = request.into_inner();
        let idx = self.module_index(req.module)?;
        let frame = req
            .frame
            .ok_or_else(|| Status::invalid_argument("missing frame"))?;
        let bytes = frame_to_bytes(&frame);

        let start = Instant::now();
        let tx_frame = PlatformRadioFrame::new_from_slice(&bytes);
        self.radios[idx]
            .lock()
            .unwrap()
            .transmit(&tx_frame)
            .map_err(|e| Status::internal(format!("transmit: {:?}", e)))?;
        let _ = self.module_tx_send.send(Box::new(TransmitModule {
            module: idx,
            frame: kaonic_ctrl::protocol::RadioFrame::new_from_frame(&tx_frame),
        }));

        Ok(Response::new(TransmitResponse {
            latency: start.elapsed().as_micros() as u32,
        }))
    }

    // ── ReceiveStream ────────────────────────────────────────────────────────

    type ReceiveStreamStream = ReceiverStream<Result<ReceiveResponse, Status>>;

    async fn receive_stream(
        &self,
        request: Request<ReceiveRequest>,
    ) -> Result<Response<Self::ReceiveStreamStream>, Status> {
        let req = request.into_inner();
        let idx = self.module_index(req.module)?;
        let proto_module = req.module;

        let mut rx = self.module_rx_send.subscribe();
        let (tx, stream_recv) = tokio::sync::mpsc::channel(16);

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if msg.module != idx {
                            continue;
                        }
                        let resp = ReceiveResponse {
                            module: proto_module,
                            frame: Some(bytes_to_frame(msg.frame.as_slice())),
                            rssi: msg.rssi as i32,
                            latency: 0,
                        };
                        if tx.send(Ok(resp)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(stream_recv)))
    }

    type TransmitEventStreamStream = ReceiverStream<Result<TransmitEventResponse, Status>>;

    async fn transmit_event_stream(
        &self,
        request: Request<TransmitEventRequest>,
    ) -> Result<Response<Self::TransmitEventStreamStream>, Status> {
        let req = request.into_inner();
        let idx = self.module_index(req.module)?;
        let proto_module = req.module;

        let mut rx = self.module_tx_send.subscribe();
        let (tx, stream_recv) = tokio::sync::mpsc::channel(16);

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if msg.module != idx {
                            continue;
                        }
                        let resp = TransmitEventResponse {
                            module: proto_module,
                            frame: Some(bytes_to_frame(msg.frame.as_slice())),
                            latency: 0,
                        };
                        if tx.send(Ok(resp)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(stream_recv)))
    }
}

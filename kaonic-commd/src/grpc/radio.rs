use kaonic_radio::{error::KaonicError, platform::kaonic1s::Kaonic1SFrame};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::kaonic::{
    radio_server::Radio, ConfigurationRequest, Empty, ReceiveRequest, ReceiveResponse,
    TransmitRequest, TransmitResponse,
};

use crate::{
    grpc::kaonic::RadioFrame,
    radio_service::{RadioService as Manager, ReceiveEvent},
};

use kaonic_radio::modulation::{
    Modulation as KrModulation, OfdmMcs, OfdmModulation, OfdmOption, QpskChipFrequency,
    QpskModulation, QpskRateMode,
};

pub struct RadioService {
    mgr: Arc<Manager>,
    shutdown: watch::Receiver<bool>,
}

impl RadioService {
    pub fn new(mgr: Arc<Manager>, shutdown: watch::Receiver<bool>) -> Self {
        Self { mgr, shutdown }
    }
}

#[tonic::async_trait]
impl Radio for RadioService {
    async fn configure(
        &self,
        request: Request<ConfigurationRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let module = module_index(req.module)?;

        let cfg = kaonic_radio::radio::RadioConfig {
            freq: req.freq * 1000,
            channel_spacing: req.channel_spacing * 1000,
            channel: req.channel as u16,
        };

        self.mgr
            .configure(module, cfg)
            .await
            .map_err(internal_err)?;

        if let Some(phy) = req.phy_config {
            log::debug!("parse modulation settings");
            let modulation = phy_to_modulation(&phy);
            if let Err(e) = modulation {
                log::error!("{}", e);
                return Err(e);
            }
            let modulation = modulation.unwrap();
            log::info!(
                "Applying modulation for module {}: {:?}",
                module,
                modulation_type_name(&modulation)
            );
            self.mgr
                .set_modulation(module, modulation)
                .await
                .map_err(internal_err)?;
        } else {
            log::warn!("no modulation settings provided");
        }

        Ok(Response::new(Empty {}))
    }

    async fn transmit(
        &self,
        request: Request<TransmitRequest>,
    ) -> Result<Response<TransmitResponse>, Status> {
        let req = request.into_inner();
        let module = module_index(req.module)?;

        if req.frame == None {
            return Err(Status::invalid_argument("frame can't be empty"));
        }

        let mut frame = Kaonic1SFrame::new();

        decode_frame(&req.frame.unwrap(), &mut frame)
            .map_err(|_| Status::resource_exhausted(""))?;

        let latency = self
            .mgr
            .transmit(module, &frame)
            .await
            .map_err(internal_err)?;

        Ok(Response::new(TransmitResponse { latency }))
    }

    type ReceiveStreamStream = ReceiverStream<Result<ReceiveResponse, Status>>;

    async fn receive_stream(
        &self,
        request: Request<ReceiveRequest>,
    ) -> Result<Response<Self::ReceiveStreamStream>, Status> {
        let req = request.into_inner();
        let module = module_index(req.module)?;

        log::debug!("start receive stream for module [{}]", module);

        // Subscribe to worker's receive broadcast and forward as gRPC stream
        let mut sub = self.mgr.subscribe(module).map_err(internal_err)?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);

        // Clone shutdown receiver for this stream
        let mut shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        break;
                    }
                    evt = sub.recv() => {
                        match evt {
                            Ok(e) => {
                                if tx.send(Ok(to_receive_response(e))).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break, // channel closed/lagged
                        }
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn module_index(module: i32) -> Result<usize, Status> {
    match module {
        0 => Ok(0), // MODULE_A
        1 => Ok(1), // MODULE_B
        x => Err(Status::invalid_argument(format!("Unknown module: {}", x))),
    }
}

fn encode_frame(buffer: &[u8]) -> RadioFrame {
    // Convert the packet bytes to a list of words
    // TODO: Optimize dynamic allocation
    let words = buffer
        .chunks(4)
        .map(|chunk| {
            let mut work = 0u32;
            let chunk = chunk.iter().as_slice();

            for i in 0..chunk.len() {
                work |= (chunk[i] as u32) << (i * 8);
            }

            work
        })
        .collect::<Vec<_>>();

    RadioFrame {
        data: words,
        length: buffer.len() as u32,
    }
}

fn decode_frame(frame: &RadioFrame, output_frame: &mut Kaonic1SFrame) -> Result<(), KaonicError> {
    if output_frame.capacity() < (frame.length as usize) {
        return Err(KaonicError::OutOfMemory);
    }

    let length = frame.length as usize;
    let mut index = 0usize;
    for word in &frame.data {
        for i in 0..4 {
            output_frame.push_data(&[((word >> i * 8) & 0xFF) as u8]);

            index += 1;

            if index >= length {
                break;
            }
        }

        if index >= length {
            break;
        }
    }

    Ok(())
}

fn to_receive_response(evt: ReceiveEvent) -> ReceiveResponse {
    super::kaonic::ReceiveResponse {
        module: evt.module as i32,
        frame: Some(encode_frame(evt.frame.as_slice())),
        rssi: evt.rssi as i32,
        latency: evt.latency_ms,
    }
}

fn internal_err<E: std::fmt::Display>(e: E) -> Status {
    Status::internal(e.to_string())
}

fn phy_to_modulation(
    phy: &super::kaonic::configuration_request::PhyConfig,
) -> Result<KrModulation, Status> {
    match phy {
        super::kaonic::configuration_request::PhyConfig::Ofdm(ofdm) => {
            let mcs = match ofdm.mcs {
                0 => OfdmMcs::Mcs0,
                1 => OfdmMcs::Mcs1,
                2 => OfdmMcs::Mcs2,
                3 => OfdmMcs::Mcs3,
                4 => OfdmMcs::Mcs4,
                5 => OfdmMcs::Mcs5,
                6 => OfdmMcs::Mcs6,
                v => return Err(Status::invalid_argument(format!("invalid OFDM mcs: {}", v))),
            };
            let opt = match ofdm.opt {
                0 => OfdmOption::Option1,
                1 => OfdmOption::Option2,
                2 => OfdmOption::Option3,
                3 => OfdmOption::Option4,
                v => {
                    return Err(Status::invalid_argument(format!(
                        "invalid OFDM option: {}",
                        v
                    )))
                }
            };
            Ok(KrModulation::Ofdm(OfdmModulation { mcs, opt }))
        }
        super::kaonic::configuration_request::PhyConfig::Qpsk(qpsk) => {
            let chip_freq = match qpsk.chip_freq {
                100 => QpskChipFrequency::Freq100,
                200 => QpskChipFrequency::Freq200,
                1000 => QpskChipFrequency::Freq1000,
                2000 => QpskChipFrequency::Freq2000,
                v => {
                    return Err(Status::invalid_argument(format!(
                        "invalid QPSK chip_freq: {}",
                        v
                    )))
                }
            };
            let mode = match qpsk.rate_mode {
                0 => QpskRateMode::Mode0,
                1 => QpskRateMode::Mode1,
                2 => QpskRateMode::Mode2,
                3 => QpskRateMode::Mode3,
                v => {
                    return Err(Status::invalid_argument(format!(
                        "invalid QPSK rate_mode: {}",
                        v
                    )))
                }
            };
            Ok(KrModulation::Qpsk(QpskModulation { chip_freq, mode }))
        }
        super::kaonic::configuration_request::PhyConfig::Fsk(_) => {
            Err(Status::unimplemented("FSK modulation not supported yet"))
        }
    }
}

fn modulation_type_name(m: &KrModulation) -> &'static str {
    match m {
        KrModulation::Ofdm(_) => "OFDM",
        KrModulation::Qpsk(_) => "QPSK",
    }
}

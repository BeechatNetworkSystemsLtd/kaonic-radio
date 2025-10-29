use kaonic_radio::{error::KaonicError, platform::kaonic1s::Kaonic1SFrame};
use std::sync::Arc;
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

pub struct RadioService {
    mgr: Arc<Manager>,
}

impl RadioService {
    pub fn new(mgr: Arc<Manager>) -> Self {
        Self { mgr }
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
            freq: req.freq,
            channel_spacing: req.channel_spacing,
            channel: req.channel as u16,
        };

        self.mgr
            .configure(module, cfg)
            .await
            .map_err(internal_err)?;

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

        tokio::spawn(async move {
            while let Ok(evt) = sub.recv().await {
                let _ = tx.send(Ok(to_receive_response(evt))).await;
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

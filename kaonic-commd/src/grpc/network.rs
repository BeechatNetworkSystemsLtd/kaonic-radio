use kaonic_radio::error::KaonicError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};

use super::kaonic::{
    network_server::Network, NetworkReceiveRequest, NetworkReceiveResponse, NetworkTransmitRequest,
    NetworkTransmitResponse,
};

use crate::{
    controller::{self, RadioController},
    grpc::kaonic::RadioFrame,
};

pub struct NetworkService {
    radio_ctrl: Arc<Mutex<RadioController>>,
    shutdown: CancellationToken,
}

impl NetworkService {
    pub fn new(radio_ctrl: Arc<Mutex<RadioController>>, shutdown: CancellationToken) -> Self {
        Self {
            radio_ctrl,
            shutdown,
        }
    }
}

#[tonic::async_trait]
impl Network for NetworkService {
    async fn transmit(
        &self,
        request: Request<NetworkTransmitRequest>,
    ) -> Result<Response<NetworkTransmitResponse>, Status> {
        let req = request.into_inner();

        if req.frame == None {
            return Err(Status::invalid_argument("frame can't be empty"));
        }

        let mut frame = controller::NetworkFrame::new();

        decode_frame(&req.frame.unwrap(), &mut frame)
            .map_err(|_| Status::resource_exhausted(""))?;

        self.radio_ctrl
            .lock()
            .await
            .network_transmit(frame)
            .map_err(|_| Status::internal("network transmit error"))?;

        Ok(Response::new(NetworkTransmitResponse { latency: 0 }))
    }

    type ReceiveStreamStream = ReceiverStream<Result<NetworkReceiveResponse, Status>>;

    async fn receive_stream(
        &self,
        _request: Request<NetworkReceiveRequest>,
    ) -> Result<Response<Self::ReceiveStreamStream>, Status> {

        log::debug!("start network receive stream");

        // Subscribe to network receive broadcast and forward as gRPC stream
        let mut sub = self.radio_ctrl.lock().await.network_receive();

        let (tx, rx) = tokio::sync::mpsc::channel(16);

        // Clone cancellation token for this stream
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        break;
                    }
                    network_recv = sub.recv() => {
                        match network_recv {
                            Ok(rx) => {
                                if tx.send(Ok(NetworkReceiveResponse {
                                            frame: Some(encode_frame(rx.frame.as_slice())),
                                            rssi: 0, // TODO: get actual RSSI if available
                                            latency: 0,
                                        }
                                    )).await.is_err() {
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

fn decode_frame(
    frame: &RadioFrame,
    output_frame: &mut controller::NetworkFrame,
) -> Result<(), KaonicError> {
    if output_frame.capacity() < (frame.length as usize) {
        return Err(KaonicError::OutOfMemory);
    }

    let length = frame.length as usize;
    let mut index = 0usize;
    for word in &frame.data {
        for i in 0..4 {
            let _ = output_frame.push_data(&[((word >> i * 8) & 0xFF) as u8]);

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

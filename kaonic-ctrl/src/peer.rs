use core::fmt;
use std::net::SocketAddr;

use kaonic_frame::frame::{Frame, FrameSegment};
use kaonic_net::{packet::AssembledPacket, request::Responder};
use rand::rngs::OsRng;
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

use crate::{error::ControllerError, network::ControllerNetwork};

pub const NETWORK_MTU: usize = 1400;

pub struct AsyncRequest<T> {
    request: oneshot::Receiver<T>,
    timeout: core::time::Duration,
}

impl<T> AsyncRequest<T> {
    pub fn new(request: oneshot::Receiver<T>, timeout: core::time::Duration) -> Self {
        Self { request, timeout }
    }

    pub async fn response(self) -> Result<T, ControllerError> {
        match tokio::time::timeout(self.timeout, self.request).await {
            Ok(response) => {
                if let Ok(response) = response {
                    return Ok(response);
                }
            }
            Err(_) => {
                return Err(ControllerError::Timeout);
            }
        }

        return Err(ControllerError::Timeout);
    }
}

pub struct AsyncResponder<T> {
    response: oneshot::Sender<T>,
}

impl<T> AsyncResponder<T> {
    pub fn new(response: oneshot::Sender<T>) -> Self {
        Self { response }
    }
}

impl<T: Copy> Responder<T> for AsyncResponder<T> {
    fn respond(self, _id: kaonic_net::packet::PacketId, response: T) {
        let _ = self.response.send(response);
    }
}

pub struct PeerMessageId(pub u32);

impl fmt::Display for PeerMessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{:0>8x}/", self.0)
    }
}

pub trait PeerMessage: Copy {
    fn message_id(&self) -> PeerMessageId;
}

pub trait PeerCoder<T: PeerMessage, const MTU: usize, const R: usize> {
    fn serialize(
        &mut self,
        message: &T,
        frame: &mut FrameSegment<MTU, R>,
    ) -> Result<(), ControllerError>;

    fn deserialize<'a>(
        &mut self,
        packet: &AssembledPacket<'a, MTU, R>,
    ) -> Result<T, ControllerError>;
}

#[derive(Clone, Copy)]
pub struct PeerTx<T: PeerMessage + Copy> {
    pub addr: Option<SocketAddr>,
    pub message: T,
}

#[derive(Clone, Copy)]
pub struct PeerRx<T: PeerMessage + Copy> {
    pub addr: SocketAddr,
    pub message: T,
}

pub type PeerSender<T> = mpsc::Sender<PeerTx<T>>;
pub type PeerReceiver<T> = broadcast::Receiver<PeerRx<T>>;

pub struct Peer<T: PeerMessage, const MTU: usize, const R: usize, C: PeerCoder<T, MTU, R>> {
    socket: UdpSocket,
    coder: C,

    network: ControllerNetwork<MTU, R>,

    frames: [Frame<MTU>; R],

    tx_frame: FrameSegment<MTU, R>,
    rx_frame: FrameSegment<MTU, R>,

    tx_send: PeerSender<T>,
    tx_recv: mpsc::Receiver<PeerTx<T>>,
    rx_send: broadcast::Sender<PeerRx<T>>,
}

impl<T: PeerMessage, const MTU: usize, const R: usize, C: PeerCoder<T, MTU, R>> Peer<T, MTU, R, C> {
    pub fn new(socket: UdpSocket, coder: C) -> Self {
        let (tx_send, tx_recv) = mpsc::channel(32);
        let (rx_send, _) = broadcast::channel(32);

        log::debug!("create new peer (mtu={},pay={})", MTU, MTU * R);

        Self {
            socket,
            coder,
            network: ControllerNetwork::new(),
            frames: core::array::from_fn(|_| Frame::new()),
            tx_frame: FrameSegment::new(),
            rx_frame: FrameSegment::new(),
            tx_send,
            tx_recv,
            rx_send,
        }
    }

    pub fn tx_send(&self) -> PeerSender<T> {
        self.tx_send.clone()
    }

    pub fn rx_recv(&self) -> PeerReceiver<T> {
        self.rx_send.subscribe()
    }

    pub async fn serve(mut self, cancel: CancellationToken) -> Result<(), ControllerError> {
        let mut recv_frame = Frame::<MTU>::new();
        let mut running = true;

        let rng = OsRng;

        loop {
            if !running {
                log::warn!("stop serving peer");
                break;
            }

            recv_frame.clear();

            tokio::select! {
                // Receive branch
                result = self.socket.recv_from(recv_frame.alloc_max_buffer()) => {
                    match result {
                        Ok((len, addr)) => {
                            recv_frame.resize(len);

                            log::trace!("socket recv {} {} B", addr, len);

                            if let Ok(packet) = self.network.receive(&recv_frame, &mut self.rx_frame) {
                                if let Ok(message) = self.coder.deserialize(&packet) {
                                    if let Err(_) = self.rx_send.send(PeerRx { addr, message }) {
                                    }
                                }
                            }

                        }
                        Err(e) => {
                            log::error!("socket error: {}", e);
                        }
                    }
                }

                // Transmit branch
                Some(tx) = self.tx_recv.recv() => {

                    match self.coder.serialize(&tx.message, &mut self.tx_frame) {
                        Ok(_) => {

                            // Split messages into segment frames
                            let segments = self.network.transmit(self.tx_frame.as_slice(), rng, &mut self.frames);

                            if let Ok(segments) = segments {
                                for (i, segment) in segments.iter().enumerate() {
                                    log::trace!("send segment[{}] {} bytes", i, segment.len());

                                    if let Some(addr) = tx.addr {
                                        if let Err(_) = self.socket.send_to(segment.as_slice(), &addr).await {
                                            log::error!("socket send error");
                                        }
                                    } else {
                                        if let Err(_) = self.socket.send(segment.as_slice()).await {
                                            log::error!("socket send error");
                                        }
                                    }
                                }
                            } else {
                                log::error!("segments were not created");
                            }
                        }
                        Err(_) => {
                            log::error!("can't serialize message");
                        }
                    }
                },

                _ = cancel.cancelled() => {
                    running = false;
                }
            }
        }

        Ok(())
    }
}

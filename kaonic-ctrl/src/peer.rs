use kaonic_frame::frame::{Frame, FrameSegment};
use kaonic_net::{packet::AssembledPacket, request::Responder};
use rand::rngs::OsRng;
use tokio::{
    io,
    net::UdpSocket,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

use crate::{error::ControllerError, network::ControllerNetwork};

pub const NETWORK_MTU: usize = 1400;

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

pub trait PeerMessage: Copy {
    fn message_id(&self) -> u32;
}

pub trait PeerCoder<T: PeerMessage, const MTU: usize, const R: usize> {
    fn serialize(&self, item: &T, frame: &mut FrameSegment<MTU, R>) -> Result<(), ControllerError>;
    fn deserialize<'a>(&self, packet: &AssembledPacket<'a, MTU, R>) -> Result<T, ControllerError>;
}

pub type PeerSender<T> = mpsc::Sender<T>;
pub type PeerReceiver<T> = broadcast::Receiver<T>;

pub struct Peer<T: PeerMessage, const MTU: usize, const R: usize, C: PeerCoder<T, MTU, R>> {
    socket: UdpSocket,
    peer_addr: String,
    coder: C,
    network: ControllerNetwork<MTU, R>,
    frames: [Frame<MTU>; R],
    tx_frame: FrameSegment<MTU, R>,
    rx_frame: FrameSegment<MTU, R>,
    tx_send: mpsc::Sender<T>,
    tx_recv: mpsc::Receiver<T>,
    rx_send: broadcast::Sender<T>,
}

impl<T: PeerMessage, const MTU: usize, const R: usize, C: PeerCoder<T, MTU, R>> Peer<T, MTU, R, C> {
    pub fn new(socket: UdpSocket, peer_addr: &str, coder: C) -> Self {
        let (tx_send, tx_recv) = mpsc::channel(32);
        let (rx_send, _) = broadcast::channel(32);

        Self {
            socket,
            coder,
            peer_addr: peer_addr.to_owned(),
            network: ControllerNetwork::new(),
            frames: [Frame::new(); _],
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

        log::debug!("peer serve");

        loop {
            if !running {
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
                                if let Ok(item) = self.coder.deserialize(&packet) {
                                    if let Err(_) = self.rx_send.send(item) {
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
                Some(message) = self.tx_recv.recv() => {

                    // if let Ok(message_frame) = encode_message(&message, &mut self.tx_frame) {
                    //
                    //     // Split messages into segment frames
                    //     let segments = self.network.transmit(message_frame.as_slice(), rng, &mut self.frames);
                    //
                    //     if let Ok(segments) = segments {
                    //         for segment in segments {
                    //             if let Err(_) = self.socket.send_to(segment.as_slice(), &self.peer_addr).await {
                    //                 log::error!("socket send error");
                    //             }
                    //         }
                    //     }
                    // }
                },

                _ = cancel.cancelled() => {
                    log::warn!("stop serving peer");
                    running = false;
                }
            }
        }

        Ok(())
    }
}

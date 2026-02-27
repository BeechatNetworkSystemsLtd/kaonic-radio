use core::fmt;
use std::{collections::HashSet, net::SocketAddr, time::Instant};

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
    request: mpsc::Receiver<T>,
    timeout: core::time::Duration,
}

impl<T> AsyncRequest<T> {
    pub fn new(request: mpsc::Receiver<T>, timeout: core::time::Duration) -> Self {
        Self { request, timeout }
    }

    pub async fn response(mut self) -> Result<T, ControllerError> {
        tokio::select! {
            biased;
            Some(message) = self.request.recv() => {
                return Ok(message)
            }
            _ = tokio::time::sleep(self.timeout) => {
                return Err(ControllerError::Timeout);
            }
        }
    }
}

pub struct AsyncResponder<T> {
    response: mpsc::Sender<T>,
}

impl<T> AsyncResponder<T> {
    pub fn new(response: mpsc::Sender<T>) -> Self {
        Self { response }
    }
}

impl<T: Clone> Responder<T> for AsyncResponder<T> {
    fn respond(self, _id: kaonic_net::packet::PacketId, response: T) {
        let _ = self.response.try_send(response);
    }
}

pub struct PeerMessageId(pub u32);

impl fmt::Display for PeerMessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{:0>8x}/", self.0)
    }
}

pub trait PeerMessage: Clone {
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

#[derive(Clone)]
pub struct PeerTx<T: PeerMessage + Clone> {
    pub time: Instant,
    pub addr: Option<SocketAddr>,
    pub message: Box<T>,
}

#[derive(Clone)]
pub struct PeerRx<T: PeerMessage + Clone> {
    pub time: Instant,
    pub addr: SocketAddr,
    pub message: Box<T>,
}

pub type PeerSender<T> = mpsc::Sender<PeerTx<T>>;
pub type PeerReceiver<T> = broadcast::Receiver<PeerRx<T>>;

#[derive(Debug)]
pub struct Peer<
    T: PeerMessage + std::fmt::Debug,
    const MTU: usize,
    const R: usize,
    C: PeerCoder<T, MTU, R> + std::fmt::Debug,
> {
    socket: UdpSocket,
    coder: C,

    filter_rx_addr: Option<SocketAddr>,
    network: ControllerNetwork<MTU, R>,

    frames: [Frame<MTU>; R],

    tx_frame: FrameSegment<MTU, R>,
    rx_frame: FrameSegment<MTU, R>,

    tx_send: PeerSender<T>,
    tx_recv: mpsc::Receiver<PeerTx<T>>,
    rx_send: broadcast::Sender<PeerRx<T>>,
}

impl<
        T: PeerMessage + std::fmt::Debug,
        const MTU: usize,
        const R: usize,
        C: PeerCoder<T, MTU, R> + std::fmt::Debug,
    > Peer<T, MTU, R, C>
{
    pub fn new(socket: UdpSocket, coder: C, filter_rx_addr: Option<SocketAddr>) -> Self {
        let (tx_send, tx_recv) = mpsc::channel(128);
        let (rx_send, _) = broadcast::channel(128);

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
            filter_rx_addr,
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

        let local_addr = self.socket.local_addr()?;

        let mut clients = HashSet::new();

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
                            if addr != local_addr && (self.filter_rx_addr.is_some_and(|a| a == addr) || self.filter_rx_addr.is_none()) {

                                clients.insert(addr);
                                recv_frame.resize(len);

                                if let Ok(packet) = self.network.receive(&recv_frame, &mut self.rx_frame) {
                                    if let Ok(message) = self.coder.deserialize(&packet) {
                                        if let Err(_) = self.rx_send.send(PeerRx { time: Instant::now(), addr, message: Box::new(message) }) {
                                        }
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

                            log::trace!("tx message time {}", tx.time.elapsed().as_millis());

                            // Split messages into segment frames
                            let segments = self.network.transmit(self.tx_frame.as_slice(), rng, &mut self.frames);

                            if let Ok(segments) = segments {
                                for segment in segments.iter() {

                                    if let Some(addr) = tx.addr {
                                        if let Err(_) = self.socket.send_to(segment.as_slice(), &addr).await {
                                            log::error!("socket send error");
                                        }
                                    } else {
                                        for addr in clients.iter().into_iter() {

                                            if let Err(_) = self.socket.send_to(segment.as_slice(), &addr).await {
                                                log::error!("socket broadcast send error");
                                            }
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

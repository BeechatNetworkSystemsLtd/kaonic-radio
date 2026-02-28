use std::{net::SocketAddr, time::Instant};

use tokio::{net::UdpSocket, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    error::ControllerError,
    peer::{Peer, PeerCoder, PeerMessage, PeerReceiver, PeerRx, PeerSender, PeerTx},
};

pub trait ServerHandler<T> {
    fn new_message(&mut self) -> Box<T>;
    fn handle_message(&mut self, request: &T, response: Box<T>) -> Option<Box<T>>;
}

pub struct Server<T: PeerMessage> {
    peer_send: PeerSender<T>,
}

impl<T: PeerMessage + Send + std::fmt::Debug + 'static> Server<T> {
    pub async fn listen<
        const MTU: usize,
        const R: usize,
        C: PeerCoder<T, MTU, R> + Send + std::fmt::Debug + 'static,
        H: ServerHandler<T> + Send + 'static,
    >(
        listen_addr: SocketAddr,
        coder: C,
        handler: H,
        client_recv: mpsc::Receiver<Box<T>>,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        log::info!("listen server on {}", listen_addr);

        let socket = UdpSocket::bind(listen_addr).await?;
        socket.set_broadcast(true)?;

        let peer = Peer::new(socket, coder, None);
        let peer_send = peer.tx_send();
        let peer_recv = peer.rx_recv();

        {
            let peer_send = peer_send.clone();
            let cancel = cancel.clone();
            tokio::spawn(Box::pin(async move {
                let _ =
                    Self::manage_requests(handler, peer_send, client_recv, peer_recv, cancel).await;
            }));
        }

        {
            let cancel = cancel.clone();
            tokio::spawn(Box::pin(async move {
                let _ = peer.serve(cancel).await;
            }));
        }

        Ok(Self { peer_send })
    }

    pub async fn broadcast(&mut self, message: T) {
        if let Err(_) = self
            .peer_send
            .send(PeerTx {
                time: Instant::now(),
                addr: None,
                message: Box::new(message),
            })
            .await
        {
            log::error!("server can't send broadcast");
        }
    }

    async fn handle_request<H: ServerHandler<T> + Send>(
        handler: &mut H,
        rx: PeerRx<T>,
        peer_send: &PeerSender<T>,
        response: Box<T>,
    ) {
        let message_id = rx.message.message_id();

        if let Some(response) = handler.handle_message(&rx.message, response) {
            let _ = peer_send
                .send(PeerTx {
                    time: rx.time,
                    addr: Some(rx.addr),
                    message: response,
                })
                .await;

            log::trace!(
                "request {} done in {} usec",
                message_id,
                rx.time.elapsed().as_micros()
            );
        }
    }

    async fn send_broadcast(message: Box<T>, peer_send: &PeerSender<T>) {
        if let Err(_) = peer_send
            .send(PeerTx {
                time: Instant::now(),
                addr: None,
                message,
            })
            .await
        {
            log::error!("server can't send broadcast");
        }
    }
    async fn manage_requests<H: ServerHandler<T> + Send>(
        mut handler: H,
        peer_send: PeerSender<T>,
        mut client_recv: mpsc::Receiver<Box<T>>,
        mut peer_recv: PeerReceiver<T>,
        cancel: CancellationToken,
    ) {
        loop {
            let response = handler.new_message();

            tokio::select! {
                biased;

                Ok(rx) = peer_recv.recv() => {
                    Self::handle_request(&mut handler, rx, &peer_send, response).await;
                },
                Some(tx) = client_recv.recv() => {
                    Self::send_broadcast(tx, &peer_send).await;
                }
                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }
}

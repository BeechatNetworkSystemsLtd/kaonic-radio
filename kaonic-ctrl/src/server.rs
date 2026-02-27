use std::{net::SocketAddr, sync::Arc, time::Instant};

use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

use crate::{
    error::ControllerError,
    peer::{AsyncRequest, Peer, PeerCoder, PeerMessage, PeerReceiver, PeerSender, PeerTx},
};

#[derive(Debug, Clone)]
pub struct ServerRequest<T: PeerMessage> {
    time: Instant,
    message: T,
    send: mpsc::Sender<Box<T>>,
}

impl<T: PeerMessage> ServerRequest<T> {
    pub fn new(time: Instant, message: T, send: mpsc::Sender<Box<T>>) -> Self {
        Self {
            time,
            message,
            send,
        }
    }

    pub async fn response(self, message: Box<T>) -> Result<(), ControllerError> {
        self.send
            .send(message)
            .await
            .map_err(|_| ControllerError::SocketError)?;

        Ok(())
    }

    pub fn time(&self) -> Instant {
        self.time
    }

    pub fn message(&self) -> &T {
        &self.message
    }
}

pub trait ServerHandler<T> {
    fn handle_message(request: &T) -> Option<T>;
}

pub struct Server<T: PeerMessage, H: ServerHandler<T>> {
    peer_send: PeerSender<T>,
    cancel: CancellationToken,
    handler: H,
}

impl<T: PeerMessage + Send + std::fmt::Debug + 'static, H: ServerHandler<T>> Server<T, H> {
    pub async fn listen<
        const MTU: usize,
        const R: usize,
        C: PeerCoder<T, MTU, R> + Send + std::fmt::Debug + 'static,
    >(
        listen_addr: SocketAddr,
        handler: H,
        coder: C,
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
            tokio::spawn(Self::manage_requests(peer_send, peer_recv, cancel));
        }

        {
            let cancel = cancel.clone();
            tokio::spawn(Box::pin(async move {
                let _ = peer.serve(cancel).await;
            }));
        }

        Ok(Self {
            peer_send,
            handler,
            cancel,
        })
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


    async fn manage_requests(
        peer_send: PeerSender<T>,
        mut peer_recv: PeerReceiver<T>,
        req_send: broadcast::Sender<ServerRequest<T>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                Ok(rx) = peer_recv.recv() => {
                    let message_id = rx.message.message_id();
                    log::trace!("server request {}", message_id);

                    let (res_send, res_recv) = mpsc::channel(1);

                    if let Ok(_) = req_send.send(ServerRequest::new(rx.time, rx.message.as_ref().clone(), res_send)) {

                        log::trace!("request {} wait {} msec", message_id, rx.time.elapsed().as_micros());
                        let request = AsyncRequest::new(res_recv, core::time::Duration::from_secs(30));
                        if let Ok(res) = request.response().await {

                            log::trace!("request {} send in {} msec", message_id, rx.time.elapsed().as_micros());

                            let _ = peer_send.send(PeerTx { time: rx.time, addr: Some(rx.addr), message: res }).await;

                            log::trace!("request {} completed in {} msec", message_id, rx.time.elapsed().as_micros());
                        } else {
                            log::error!("request wasn't handled");
                        }
                    }
                },
                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }
}

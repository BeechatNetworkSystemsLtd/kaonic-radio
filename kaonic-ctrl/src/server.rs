use std::net::SocketAddr;

use tokio::{
    net::UdpSocket,
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

use crate::{
    error::ControllerError,
    peer::{AsyncRequest, Peer, PeerCoder, PeerMessage, PeerReceiver, PeerSender, PeerTx},
};

pub struct ServerRequest<T: PeerMessage> {
    message: T,
    send: oneshot::Sender<T>,
}

impl<T: PeerMessage> ServerRequest<T> {
    pub fn new(message: T, send: oneshot::Sender<T>) -> Self {
        Self { message, send }
    }

    pub fn response(self, message: T) -> Result<(), ControllerError> {
        self.send
            .send(message)
            .map_err(|_| ControllerError::SocketError)
    }

    pub fn message(&self) -> &T {
        &self.message
    }
}

pub struct Server<T: PeerMessage> {
    req_recv: mpsc::Receiver<ServerRequest<T>>,
    cancel: CancellationToken,
}

impl<T: PeerMessage + Send + 'static> Server<T> {
    pub async fn listen<
        const MTU: usize,
        const R: usize,
        C: PeerCoder<T, MTU, R> + Send + 'static,
    >(
        listen_addr: SocketAddr,
        coder: C,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        let (req_send, req_recv) = mpsc::channel(8);

        log::info!("listen server on {}", listen_addr);

        let socket = UdpSocket::bind(listen_addr).await?;
        socket.set_broadcast(true)?;

        let peer = Peer::new(socket, coder);
        let peer_send = peer.tx_send();
        let peer_recv = peer.rx_recv();

        {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _ = peer.serve(cancel).await;
            });
        }

        tokio::spawn(Self::manage_requests(
            peer_send,
            peer_recv,
            req_send.clone(),
            cancel.clone(),
        ));

        Ok(Self { req_recv, cancel })
    }

    /// Wait of a next request
    pub async fn request(&mut self) -> Result<ServerRequest<T>, ControllerError> {
        match self.req_recv.recv().await {
            Some(r) => Ok(r),
            None => Err(ControllerError::SocketError),
        }
    }

    pub fn cancel(&mut self) {
        self.cancel.cancel();
    }

    async fn manage_requests(
        peer_send: PeerSender<T>,
        mut peer_recv: PeerReceiver<T>,
        req_send: mpsc::Sender<ServerRequest<T>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                Ok(rx) = peer_recv.recv() => {
                    let (res_send, res_recv) = oneshot::channel();

                    log::trace!("server request {}", rx.message.message_id());

                    if let Ok(_) = req_send.send(ServerRequest::new(rx.message, res_send)).await {
                        let request = AsyncRequest::new(res_recv, core::time::Duration::from_secs(30));
                        if let Ok(res) = request.response().await {
                            let _ = peer_send.send(PeerTx { addr: rx.addr, message: res }).await;
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

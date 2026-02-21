use std::{net::SocketAddr, sync::Arc};

use kaonic_net::request::RequestQueue;
use rand::{rngs::OsRng, RngCore};
use tokio::{
    net::UdpSocket,
    sync::{broadcast, oneshot, Mutex},
};
use tokio_util::sync::CancellationToken;

use crate::{
    error::ControllerError,
    peer::{
        AsyncRequest, AsyncResponder, Peer, PeerCoder, PeerMessage, PeerReceiver, PeerSender,
        PeerTx,
    },
};

pub type ClientRequestQueue<T> = RequestQueue<16, T, AsyncResponder<T>>;

pub struct Client<T: PeerMessage> {
    tx_send: PeerSender<T>,
    rx_send: broadcast::Sender<T>,
    server_addr: SocketAddr,
    cancel: CancellationToken,
    request_queue: Arc<Mutex<ClientRequestQueue<T>>>,
}

impl<T: PeerMessage + Send + 'static> Client<T> {
    pub async fn connect<
        const MTU: usize,
        const R: usize,
        C: PeerCoder<T, MTU, R> + Send + 'static,
    >(
        listen_addr: SocketAddr,
        server_addr: SocketAddr,
        coder: C,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        let request_queue = Arc::new(Mutex::new(RequestQueue::new()));

        log::debug!(
            "client connect to {} and listen {}",
            server_addr,
            listen_addr
        );

        let socket = UdpSocket::bind(listen_addr).await?;

        let peer = Peer::new(socket, coder);
        let peer_send = peer.tx_send();
        let peer_recv = peer.rx_recv();

        {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _ = peer.serve(cancel).await;
            });
        }

        let (rx_send, _) = broadcast::channel(16);

        tokio::spawn(Self::manage_responses(
            request_queue.clone(),
            rx_send.clone(),
            peer_recv,
            cancel.clone(),
        ));

        Ok(Self {
            tx_send: peer_send,
            rx_send,
            server_addr,
            cancel,
            request_queue,
        })
    }

    pub fn receive(&self) -> broadcast::Receiver<T> {
        self.rx_send.subscribe()
    }

    pub async fn request(
        &mut self,
        message: T,
        timeout: core::time::Duration,
    ) -> Result<T, ControllerError> {
        let (res_send, res_recv) = oneshot::channel();

        self.request_queue.lock().await.request(
            message.message_id().0,
            crate::system_time(),
            timeout,
            AsyncResponder::new(res_send),
        )?;

        if let Err(_) = self
            .tx_send
            .send(PeerTx {
                addr: Some(self.server_addr),
                message,
            })
            .await
        {
            log::error!("can't send message");
            return Err(ControllerError::SocketError);
        }

        AsyncRequest::new(res_recv, timeout).response().await
    }

    pub fn cancel(&mut self) {
        self.cancel.cancel();
    }

    async fn manage_responses(
        request_queue: Arc<Mutex<ClientRequestQueue<T>>>,
        rx_send: broadcast::Sender<T>,
        mut recv: PeerReceiver<T>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                Ok(rx) = recv.recv() => {
                    request_queue.lock().await.response(rx.message.message_id().0, rx.message);

                    if let Err(_) = rx_send.send(rx.message) {
                        log::error!("client receive error");
                    }
                },
                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }

    pub fn gen_id(&self) -> u32 {
        let mut rng = OsRng;
        rng.next_u32()
    }
}

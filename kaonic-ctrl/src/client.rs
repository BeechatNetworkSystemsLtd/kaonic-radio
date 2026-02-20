use std::sync::Arc;

use kaonic_net::request::{self, RequestQueue};
use radio_common::Modulation;
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc, oneshot, Mutex},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::{
    error::ControllerError,
    peer::{AsyncResponder, PeerMessage, PeerReceiver, PeerSender},
};

pub type ClientRequestQueue<T> = RequestQueue<16, T, AsyncResponder<T>>;

pub struct Client<T: PeerMessage, const MTU: usize, const R: usize> {
    tx_send: mpsc::Sender<T>,
    cancel: CancellationToken,
    request_queue: Arc<Mutex<ClientRequestQueue<T>>>,
}

impl<T: PeerMessage + Send + 'static, const MTU: usize, const R: usize> Client<T, MTU, R> {
    pub async fn new(
        peer_send: PeerSender<T>,
        peer_recv: PeerReceiver<T>,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        // Setup peer

        let request_queue = Arc::new(Mutex::new(RequestQueue::new()));

        tokio::spawn(Self::manage_responses(
            request_queue.clone(),
            peer_recv,
            cancel.clone(),
        ));

        Ok(Self {
            tx_send: peer_send,
            cancel,
            request_queue,
        })
    }

    pub async fn request(
        &mut self,
        message: T,
        timeout: core::time::Duration,
    ) -> Result<T, ControllerError> {
        let (res_send, res_recv) = oneshot::channel();

        self.request_queue.lock().await.request(
            message.message_id(),
            crate::system_time(),
            timeout,
            AsyncResponder::new(res_send),
        )?;

        if let Err(_) = self.tx_send.send(message).await {
            log::error!("can't send message");
            return Err(ControllerError::SocketError);
        }

        match tokio::time::timeout(timeout, res_recv).await {
            Ok(response) => {
                if let Ok(response) = response {
                    return Ok(response);
                }
            }
            Err(_) => {
                return Err(ControllerError::Timeout);
            }
        }

        Err(ControllerError::Timeout)
    }

    async fn manage_responses(
        request_queue: Arc<Mutex<ClientRequestQueue<T>>>,
        mut recv: PeerReceiver<T>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                Ok(item) = recv.recv() => {
                    request_queue.lock().await.response(0, item);
                },
                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }
}

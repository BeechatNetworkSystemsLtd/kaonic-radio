use std::{net::SocketAddr, time::Duration};

use kaonic_frame::frame::Frame;
use rand::rngs::OsRng;
use radio_common::{Modulation, RadioConfig};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    error::ControllerError,
    peer::{PeerSender, PeerTx},
    protocol::{
        Message, MessageBuilder, Payload, RADIO_FRAME_SIZE, RadioFrame, ReceiveModule,
        TransmitModule,
    },
};

pub use crate::protocol::GetInfoResponse;

/// Default timeout for all request/response operations.
pub const DEFAULT_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(6);
const KEEPALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MODULE_EVENT_CHANNEL_CAPACITY: usize = 300;

/// High-level client for interacting with a remote radio device over the kaonic-ctrl protocol.
///
/// Wraps a [`Client`] and provides typed async methods for transmitting frames,
/// querying/configuring radio modules, and receiving incoming frames via a broadcast channel.
pub struct RadioClient {
    module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
    module_tx_send: broadcast::Sender<Box<TransmitModule>>,
    activity_send: watch::Sender<u64>,
    cancel: CancellationToken,
    client: Client<Message>,
    timeout: core::time::Duration,
}

impl RadioClient {
    /// Creates a new `RadioClient` from an established [`Client`] connection.
    ///
    /// Spawns a background task that forwards incoming [`ReceiveModule`] payloads
    /// to subscribers via a broadcast channel. The task is cancelled when `cancel` is triggered.
    pub async fn new(
        client: Client<Message>,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        Self::new_with_keepalive_timeout(client, cancel, KEEPALIVE_IDLE_TIMEOUT).await
    }

    async fn new_with_keepalive_timeout(
        client: Client<Message>,
        cancel: CancellationToken,
        keepalive_timeout: Duration,
    ) -> Result<Self, ControllerError> {
        let rx_recv = client.receive();
        let keepalive_send = client.tx_sender();
        let server_addr = client.server_addr();

        let (module_rx_send, _) = broadcast::channel(MODULE_EVENT_CHANNEL_CAPACITY);
        let (module_tx_send, _) = broadcast::channel(MODULE_EVENT_CHANNEL_CAPACITY);
        let (activity_send, activity_recv) = watch::channel(0u64);

        tokio::spawn(Self::listen_events(
            rx_recv,
            module_rx_send.clone(),
            module_tx_send.clone(),
            activity_send.clone(),
            cancel.clone(),
        ));

        tokio::spawn(Self::keepalive_task(
            keepalive_send,
            server_addr,
            activity_send.clone(),
            activity_recv,
            cancel.clone(),
            keepalive_timeout,
        ));

        Ok(Self {
            module_rx_send,
            module_tx_send,
            activity_send,
            client,
            cancel,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Sets the timeout used for all request/response operations.
    pub fn set_timeout(&mut self, timeout: core::time::Duration) {
        self.timeout = timeout;
    }

    /// Returns a broadcast receiver that yields incoming [`ReceiveModule`] frames
    /// from all radio modules. Multiple callers can each subscribe independently.
    pub fn module_receive(&self) -> broadcast::Receiver<Box<ReceiveModule>> {
        self.module_rx_send.subscribe()
    }

    /// Returns a broadcast receiver that yields outgoing [`TransmitModule`] frames
    /// from all radio modules. Multiple callers can each subscribe independently.
    pub fn module_transmit(&self) -> broadcast::Receiver<Box<TransmitModule>> {
        self.module_tx_send.subscribe()
    }

    /// Sends a ping to the device and waits for a pong response.
    ///
    /// Useful for verifying that the connection is alive.
    pub async fn ping(&mut self) -> Result<(), ControllerError> {
        let response = self.request(Payload::Ping).await?;

        match response.payload {
            Payload::Pong => Ok(()),
            _ => Err(ControllerError::DecodeError),
        }
    }

    /// Transmits a frame through the specified radio module.
    pub async fn transmit(
        &mut self,
        module: usize,
        frame: &Frame<RADIO_FRAME_SIZE>,
    ) -> Result<(), ControllerError> {
        let response = self
            .request(Payload::TransmitModuleRequest(crate::protocol::TransmitModule {
                module,
                frame: RadioFrame::new_from_frame(frame),
            }))
            .await?;

        match response.payload {
            Payload::Error => Err(ControllerError::MethodError),
            Payload::TransmitModuleResponse => Ok(()),
            _ => Err(ControllerError::DecodeError),
        }
    }

    /// Sets the modulation scheme for the specified radio module.
    pub async fn set_modulation(
        &mut self,
        module: usize,
        modulation: Modulation,
    ) -> Result<(), ControllerError> {
        self.request(Payload::SetModulationRequest(
            crate::protocol::SetModulationRequest { module, modulation },
        ))
        .await?;

        Ok(())
    }

    /// Retrieves the current modulation scheme of the specified radio module.
    pub async fn get_modulation(&mut self, module: usize) -> Result<Modulation, ControllerError> {
        let response = self
            .request(Payload::GetModulationRequest(
                crate::protocol::GetModulationRequest { module },
            ))
            .await?;

        match response.payload {
            Payload::Error => Err(ControllerError::MethodError),
            Payload::GetModulationResponse(r) => Ok(r.modulation),
            _ => Err(ControllerError::DecodeError),
        }
    }

    /// Retrieves the current radio configuration of the specified module.
    pub async fn get_radio_config(
        &mut self,
        module: usize,
    ) -> Result<RadioConfig, ControllerError> {
        let response = self
            .request(Payload::GetRadioConfigRequest(
                crate::protocol::GetRadioConfigRequest { module },
            ))
            .await?;

        match response.payload {
            Payload::Error => Err(ControllerError::MethodError),
            Payload::GetRadioConfigResponse(r) => Ok(r.config),
            _ => Err(ControllerError::DecodeError),
        }
    }

    /// Applies a new radio configuration to the specified module.
    pub async fn set_radio_config(
        &mut self,
        module: usize,
        config: RadioConfig,
    ) -> Result<(), ControllerError> {
        self.request(Payload::SetRadioConfigRequest(
            crate::protocol::SetRadioConfigRequest { module, config },
        ))
        .await?;

        Ok(())
    }

    /// Queries the device for general info (e.g. number of radio modules).
    pub async fn get_info(&mut self) -> Result<GetInfoResponse, ControllerError> {
        let response = self.request(Payload::GetInfoRequest).await?;

        match response.payload {
            Payload::Error => Err(ControllerError::MethodError),
            Payload::GetInfoResponse(info) => Ok(info),
            _ => Err(ControllerError::DecodeError),
        }
    }

    /// Cancels the background receive task and shuts down the underlying client.
    pub fn cancel(&mut self) {
        self.client.cancel();
        self.cancel.cancel();
    }

    async fn request(&mut self, payload: Payload) -> Result<Message, ControllerError> {
        self.touch_activity();

        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(payload)
                    .build(),
                self.timeout,
            )
            .await
    }

    fn touch_activity(&self) {
        Self::touch_watch(&self.activity_send);
    }

    fn touch_watch(activity_send: &watch::Sender<u64>) {
        let next = (*activity_send.borrow()).wrapping_add(1);
        let _ = activity_send.send(next);
    }

    async fn listen_events(
        mut rx_recv: broadcast::Receiver<Box<Message>>,
        module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
        module_tx_send: broadcast::Sender<Box<TransmitModule>>,
        activity_send: watch::Sender<u64>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                recv = rx_recv.recv() => match recv {
                    Ok(message) => {
                        match message.payload {
                            Payload::ReceiveModule(rx) => {
                                let _ = module_rx_send.send(Box::new(rx));
                            },
                            Payload::TransmitModuleEvent(tx) => {
                                let _ = module_tx_send.send(Box::new(tx));
                            },
                            _ => {}
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("radio client event stream lagged by {skipped} messages");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn keepalive_task(
        keepalive_send: PeerSender<Message>,
        server_addr: SocketAddr,
        activity_send: watch::Sender<u64>,
        mut activity_recv: watch::Receiver<u64>,
        cancel: CancellationToken,
        keepalive_timeout: Duration,
    ) {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    break;
                }
                changed = tokio::time::timeout(keepalive_timeout, activity_recv.changed()) => match changed {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => break,
                    Err(_) => {
                        Self::touch_watch(&activity_send);

                        let ping = MessageBuilder::new()
                            .with_rnd_id(OsRng)
                            .with_payload(Payload::Ping)
                            .build();

                        if let Err(_) = keepalive_send.send(PeerTx {
                            time: std::time::Instant::now(),
                            addr: Some(server_addr),
                            message: Box::new(ping),
                        }).await {
                            log::warn!("radio client keepalive send failed");
                        }
                    }
                }
            }
        }
    }
}

//! Client-side handles for a channel: a transmit half, a receive half, and the
//! builder that opens one on the daemon.
//!
//! The handles hold no coder and no buffers — a channel number, a client
//! handle and a subscription. All the work happens in the daemon.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::broadcast;

use crate::client::Client;
use crate::error::ControllerError;
use crate::peer::PeerTx;
use crate::protocol::{
    ChannelPeerRequest, ChannelSendRequest, ChannelSendResponse, ChannelStatsRequest,
    CloseChannelRequest, Message, MessageBuilder, OpenChannelRequest, OpenChannelResponse,
    Payload,
};
use crate::radio::RadioClient;

use super::spec::{
    ChannelEvent, ChannelId, ChannelLimits, ChannelStats, PeerKey, ProfileSpec, RxInfo,
    TrafficClass, TxHint,
};

/// Payloads buffered per subscriber before the oldest is dropped.
const RECEIVE_CAPACITY: usize = 256;
const EVENT_CAPACITY: usize = 64;
/// How long `send` waits between attempts while the daemon reports a full queue.
const QUEUE_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub enum ChannelError {
    PayloadTooLarge { len: usize, max: usize },
    QueueFull,
    Closed,
    Refused(String),
    Radio(ControllerError),
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelError::PayloadTooLarge { len, max } => {
                write!(f, "payload is {len} bytes, channel accepts at most {max}")
            }
            ChannelError::QueueFull => write!(f, "channel queue is full"),
            ChannelError::Closed => write!(f, "channel is closed"),
            ChannelError::Refused(reason) => write!(f, "daemon refused to open the channel: {reason}"),
            ChannelError::Radio(err) => write!(f, "radio daemon: {err:?}"),
        }
    }
}

impl std::error::Error for ChannelError {}

impl From<ControllerError> for ChannelError {
    fn from(err: ControllerError) -> Self {
        ChannelError::Radio(err)
    }
}

/// A payload delivered by the daemon.
#[derive(Debug, Clone)]
pub struct Received {
    pub payload: Bytes,
    pub info: RxInfo,
}

/// Where the client's event pump delivers a channel's payloads and events.
pub(crate) struct Route {
    pub payloads: broadcast::Sender<Received>,
    pub events: broadcast::Sender<ChannelEvent>,
}

/// Registry of open channels, shared between the handles and the client's
/// event pump. Keyed by id: several handles on one id share the route.
#[derive(Clone, Default)]
pub(crate) struct ChannelRouter {
    routes: Arc<Mutex<HashMap<ChannelId, Route>>>,
}

impl ChannelRouter {
    fn subscribe(&self, id: ChannelId) -> (broadcast::Receiver<Received>, broadcast::Sender<ChannelEvent>) {
        let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        let route = routes.entry(id).or_insert_with(|| Route {
            payloads: broadcast::channel(RECEIVE_CAPACITY).0,
            events: broadcast::channel(EVENT_CAPACITY).0,
        });
        (route.payloads.subscribe(), route.events.clone())
    }

    fn forget(&self, id: ChannelId) {
        let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        routes.remove(&id);
    }

    /// Called by the event pump for every `ChannelReceive`.
    pub(crate) fn deliver(&self, id: ChannelId, payload: Vec<u8>, info: RxInfo) {
        let routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(route) = routes.get(&id) {
            let _ = route.payloads.send(Received {
                payload: Bytes::from(payload),
                info,
            });
        }
    }

    /// Called by the event pump for every `ChannelEvent`.
    pub(crate) fn notify(&self, id: ChannelId, event: ChannelEvent) {
        let routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(route) = routes.get(&id) {
            let _ = route.events.send(event);
        }
    }
}

/// State shared by every handle of one opened channel. Dropping the last
/// handle closes the channel on the daemon.
struct Shared {
    id: ChannelId,
    client: Client<Message>,
    router: ChannelRouter,
    limits: ChannelLimits,
    events: broadcast::Sender<ChannelEvent>,
    timeout: Duration,
    closed: AtomicBool,
}

impl Shared {
    async fn request(&self, payload: Payload) -> Result<Message, ChannelError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ChannelError::Closed);
        }
        let message = MessageBuilder::new()
            .with_id(self.client.gen_id())
            .with_payload(payload)
            .build();
        Ok(self.client.request(message, self.timeout).await?)
    }

    /// Sends without waiting for a reply; for paths that must not block.
    fn post(&self, payload: Payload) -> Result<(), ChannelError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ChannelError::Closed);
        }
        let message = MessageBuilder::new()
            .with_id(self.client.gen_id())
            .with_payload(payload)
            .build();
        self.client
            .tx_sender()
            .try_send(PeerTx {
                time: std::time::Instant::now(),
                addr: Some(self.client.server_addr()),
                message: Box::new(message),
            })
            .map_err(|_| ChannelError::QueueFull)
    }

    async fn stats(&self) -> Result<ChannelStats, ChannelError> {
        let reply = self
            .request(Payload::ChannelStatsRequest(ChannelStatsRequest { id: self.id }))
            .await?;
        match reply.payload {
            Payload::ChannelStatsResponse(stats) => Ok(stats.stats),
            _ => Err(ChannelError::Radio(ControllerError::DecodeError)),
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.router.forget(self.id);
        let message = MessageBuilder::new()
            .with_id(self.client.gen_id())
            .with_payload(Payload::CloseChannelRequest(CloseChannelRequest { id: self.id }))
            .build();
        let _ = self.client.tx_sender().try_send(PeerTx {
            time: std::time::Instant::now(),
            addr: Some(self.client.server_addr()),
            message: Box::new(message),
        });
    }
}

/// Transmit half of a channel. Cheap to clone; every clone sends into the same
/// daemon-side queue.
#[derive(Clone)]
pub struct ChannelTx {
    shared: Arc<Shared>,
}

impl ChannelTx {
    pub fn id(&self) -> ChannelId {
        self.shared.id
    }

    /// Largest single payload the channel accepts.
    pub fn mtu(&self) -> usize {
        self.shared.limits.mtu as usize
    }

    /// Payload bytes one radio frame carries under the channel's current code.
    /// Payloads sized to a multiple of this waste nothing on padding.
    pub fn frame_capacity(&self) -> usize {
        self.shared.limits.frame_capacity as usize
    }

    /// Queues `payload`; waits while the daemon's queue for this channel is
    /// full (a `Block` queue) or returns as soon as it is accepted.
    pub async fn send(&self, payload: &[u8]) -> Result<(), ChannelError> {
        self.send_with(payload, TxHint::default()).await
    }

    /// Like [`send`](Self::send), with what the protocol knows about the frame:
    /// the peer it is for and, optionally, a traffic class for this frame only.
    pub async fn send_with(&self, payload: &[u8], hint: TxHint) -> Result<(), ChannelError> {
        self.check_len(payload.len())?;
        let hint = Some(TxHint {
            len: payload.len(),
            ..hint
        });
        loop {
            let reply = self
                .shared
                .request(Payload::ChannelSendRequest(ChannelSendRequest {
                    id: self.shared.id,
                    payload: payload.to_vec(),
                    hint,
                }))
                .await?;
            match reply.payload {
                Payload::ChannelSendResponse(ChannelSendResponse::Accepted) => return Ok(()),
                Payload::ChannelSendResponse(ChannelSendResponse::QueueFull) => {
                    tokio::time::sleep(QUEUE_RETRY).await;
                }
                Payload::ChannelSendResponse(ChannelSendResponse::Oversize { max }) => {
                    return Err(ChannelError::PayloadTooLarge {
                        len: payload.len(),
                        max: max as usize,
                    })
                }
                Payload::ChannelSendResponse(ChannelSendResponse::Closed) => {
                    return Err(ChannelError::Closed)
                }
                _ => return Err(ChannelError::Radio(ControllerError::DecodeError)),
            }
        }
    }

    /// Hands `payload` to the daemon without waiting for anything. For
    /// real-time producers on a `DropOldest` queue, where a payload that
    /// cannot go now is better dropped than delivered late.
    pub fn try_send(&self, payload: &[u8]) -> Result<(), ChannelError> {
        self.try_send_with(payload, TxHint::default())
    }

    /// Like [`try_send`](Self::try_send), with what the protocol knows about
    /// the frame.
    pub fn try_send_with(&self, payload: &[u8], hint: TxHint) -> Result<(), ChannelError> {
        self.check_len(payload.len())?;
        self.shared
            .post(Payload::ChannelSendRequest(ChannelSendRequest {
                id: self.shared.id,
                payload: payload.to_vec(),
                hint: Some(TxHint {
                    len: payload.len(),
                    ..hint
                }),
            }))
    }

    /// Tells the daemon what the protocol learned about `peer` from a payload
    /// it parsed: the RSSI it arrived with, and/or the traffic class to use
    /// toward it from now on. Drives adaptive coding.
    pub fn observe_peer(&self, peer: PeerKey, rssi: Option<i8>, class: Option<TrafficClass>) {
        let _ = self.shared.post(Payload::ChannelPeerRequest(ChannelPeerRequest {
            id: self.shared.id,
            peer,
            rssi,
            class,
        }));
    }

    pub async fn stats(&self) -> Result<ChannelStats, ChannelError> {
        self.shared.stats().await
    }

    /// Daemon-side events for this channel.
    pub fn events(&self) -> broadcast::Receiver<ChannelEvent> {
        self.shared.events.subscribe()
    }

    fn check_len(&self, len: usize) -> Result<(), ChannelError> {
        let max = self.mtu();
        if len == 0 || len > max {
            return Err(ChannelError::PayloadTooLarge { len, max });
        }
        Ok(())
    }
}

/// Receive half of a channel: a stream of payloads the daemon decoded for it.
pub struct ChannelRx {
    shared: Arc<Shared>,
    recv: broadcast::Receiver<Received>,
}

impl ChannelRx {
    pub fn id(&self) -> ChannelId {
        self.shared.id
    }

    /// Next payload. `Err(Closed)` once the channel is gone.
    pub async fn recv(&mut self) -> Result<Received, ChannelError> {
        loop {
            match self.recv.recv().await {
                Ok(received) => return Ok(received),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("channel {}: receiver lagged, {n} payload(s) dropped", self.shared.id.raw());
                }
                Err(broadcast::error::RecvError::Closed) => return Err(ChannelError::Closed),
            }
        }
    }

    /// A payload if one is waiting; never blocks.
    pub fn try_recv(&mut self) -> Result<Option<Received>, ChannelError> {
        match self.recv.try_recv() {
            Ok(received) => Ok(Some(received)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Lagged(_)) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => Err(ChannelError::Closed),
        }
    }

    pub async fn stats(&self) -> Result<ChannelStats, ChannelError> {
        self.shared.stats().await
    }

    /// A second, independent consumer of the same stream.
    pub fn resubscribe(&self) -> ChannelRx {
        ChannelRx {
            shared: self.shared.clone(),
            recv: self.recv.resubscribe(),
        }
    }

    pub fn events(&self) -> broadcast::Receiver<ChannelEvent> {
        self.shared.events.subscribe()
    }
}

/// Both halves of a freshly opened channel.
pub struct Channel {
    tx: ChannelTx,
    rx: ChannelRx,
}

impl Channel {
    pub fn split(self) -> (ChannelTx, ChannelRx) {
        (self.tx, self.rx)
    }

    pub fn tx(&self) -> &ChannelTx {
        &self.tx
    }

    pub fn rx(&mut self) -> &mut ChannelRx {
        &mut self.rx
    }

    pub fn id(&self) -> ChannelId {
        self.tx.id()
    }
}

/// Opens a channel on the daemon. Owns cheap handles, so it can be kept, sent
/// to another task, and awaited without borrowing the client.
pub struct ChannelBuilder {
    client: Client<Message>,
    router: ChannelRouter,
    id: ChannelId,
    modules: Vec<u8>,
    profile: ProfileSpec,
    timeout: Duration,
}

impl ChannelBuilder {
    pub(crate) fn new(client: &RadioClient, id: ChannelId, timeout: Duration) -> Self {
        Self {
            client: client.client_handle(),
            router: client.channel_router(),
            id,
            modules: vec![0],
            profile: ProfileSpec::default(),
            timeout,
        }
    }

    /// Radio modules the channel uses; one for a plain channel, several for
    /// bonding or diversity as the profile's dispatch decides.
    pub fn modules(mut self, modules: impl IntoIterator<Item = usize>) -> Self {
        self.modules = modules.into_iter().map(|m| m as u8).collect();
        self
    }

    /// How the channel uses the air. Defaults to `Robust`, today's behaviour.
    pub fn profile(mut self, profile: impl Into<ProfileSpec>) -> Self {
        self.profile = profile.into();
        self
    }

    pub async fn build(self) -> Result<Channel, ChannelError> {
        let (shared, recv) = self.open(true, true).await?;
        Ok(Channel {
            tx: ChannelTx {
                shared: shared.clone(),
            },
            rx: ChannelRx { shared, recv },
        })
    }

    /// Transmit only: the daemon keeps no receive state for this client.
    pub async fn build_tx(self) -> Result<ChannelTx, ChannelError> {
        let (shared, _) = self.open(true, false).await?;
        Ok(ChannelTx { shared })
    }

    /// Receive only.
    pub async fn build_rx(self) -> Result<ChannelRx, ChannelError> {
        let (shared, recv) = self.open(false, true).await?;
        Ok(ChannelRx { shared, recv })
    }

    async fn open(self, tx: bool, rx: bool) -> Result<(Arc<Shared>, broadcast::Receiver<Received>), ChannelError> {
        let client = self.client;
        let router = self.router;
        let (recv, events) = router.subscribe(self.id);

        let message = MessageBuilder::new()
            .with_id(client.gen_id())
            .with_payload(Payload::OpenChannelRequest(OpenChannelRequest {
                id: self.id,
                modules: self.modules,
                profile: self.profile,
                tx,
                rx,
            }))
            .build();
        let reply = match client.request(message, self.timeout).await {
            Ok(reply) => reply,
            Err(err) => {
                router.forget(self.id);
                return Err(err.into());
            }
        };
        let limits = match reply.payload {
            Payload::OpenChannelResponse(OpenChannelResponse::Opened(limits)) => limits,
            Payload::OpenChannelResponse(OpenChannelResponse::Refused(reason)) => {
                router.forget(self.id);
                return Err(ChannelError::Refused(reason));
            }
            _ => {
                router.forget(self.id);
                return Err(ChannelError::Radio(ControllerError::DecodeError));
            }
        };

        let shared = Arc::new(Shared {
            id: self.id,
            client,
            router,
            limits,
            events,
            timeout: self.timeout,
            closed: AtomicBool::new(false),
        });
        Ok((shared, recv))
    }
}

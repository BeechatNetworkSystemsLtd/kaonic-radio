//! The channels open on one module, shared between the ctrl handler (which
//! opens, closes and enqueues) and the engine thread (which drains, codes and
//! delivers). Coders live in the engine, not here, so a decode never holds
//! this lock.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kaonic_ctrl::channel::spec::{
    ChannelId, ChannelLimits, ChannelStats, PeerKey, ProfileSpec, QueuePolicy, TrafficClass,
    TxHint,
};
use kaonic_ctrl::protocol::{ChannelSendResponse, CHANNEL_MAX_PAYLOAD, RADIO_FRAME_SIZE};
use kaonic_net::coder::{FecCode, LdpcPacketCoder};

use super::bundler::Bundler;
use super::profile::{self, Profile};

/// Radio frames one framed payload may span on a single-module channel. This
/// is what every node on the air uses today, so it must not change.
pub const SEGMENTS: usize = 3;
/// Radio frames a bonded payload may span. The packet header carries `seq` and
/// `seq_count` in four bits each, so fifteen is the wire maximum; six keeps a
/// whole payload inside the ctrl link's message size, which is what actually
/// binds. Both ends of a bonded channel opt in, so raising it past [`SEGMENTS`]
/// costs nothing in compatibility.
pub const BONDED_SEGMENTS: usize = 6;
/// Reassembly slots per channel on a single-module table: payloads that may be
/// partway through arriving at once. A slot is held from the first frame to
/// the last or until the reassembly window lapses, and the far side may have
/// several payloads interleaved on the air, so four was too few: when they
/// were all taken the newest payload was thrown away whole.
pub const SLOTS: usize = 8;
/// Reassembly slots per channel on the bonded table. Two radios deliver two
/// payloads' frames interleaved, and a fast radio runs ahead of the slow one,
/// so twice as many payloads are partway through at any moment.
pub const BONDED_SLOTS: usize = 16;
/// Payload bytes one frame carries under the legacy-compatible segment size.
pub const FRAME_CAPACITY: usize = LdpcPacketCoder::<RADIO_FRAME_SIZE>::max_payload_for(FecCode::Tm2048);
/// Largest single framed payload on a single-module channel.
pub const FRAMED_MTU: usize = FRAME_CAPACITY * SEGMENTS;
/// Largest PSDU the radio takes; the Raw MTU.
pub const RAW_MTU: usize = RADIO_FRAME_SIZE - 1;

/// An opener that has not been heard from for this long is gone; its channel
/// closes when it was the last one.
pub const OPENER_TTL: Duration = Duration::from_secs(90);

pub struct Queued {
    pub payload: Vec<u8>,
    pub hint: TxHint,
}

pub struct Opener {
    pub tx: bool,
    pub rx: bool,
    pub last_seen: Instant,
}

pub struct ChannelState {
    pub id: ChannelId,
    pub profile: Arc<dyn Profile>,
    pub openers: HashMap<SocketAddr, Opener>,
    pub queue: VecDeque<Queued>,
    pub bundler: Bundler,
    pub limits: ChannelLimits,
    pub stats: ChannelStats,
    /// Frames sent in the current burst.
    pub burst: u8,
    /// The channel yields the air until then.
    pub rest_until: Option<Instant>,
}

/// Bytes a bundle may hold: two frames' worth of payload, or the MTU when that
/// is smaller (a single-module channel under bundling).
pub fn bundle_capacity(limits: &ChannelLimits) -> usize {
    (limits.frame_capacity as usize * 2).min(limits.mtu as usize)
}

/// Bundling is part of the profile, except on the default channel, whose
/// payloads must stay byte-identical to what older nodes expect. Sizing (in
/// [`ChannelState::new`]) and dispatch (in [`ChannelState::bundling`]) must
/// agree, so both ask here.
fn bundling_enabled(raw: bool, profile: &dyn Profile, id: ChannelId) -> bool {
    !raw && profile.tdd().bundle_window > Duration::ZERO && id != ChannelId::DEFAULT
}

impl ChannelState {
    /// `segments` is how many radio frames one framed payload may span, and so
    /// what the channel's MTU is; the table it belongs to decides it.
    fn new(id: ChannelId, spec: &ProfileSpec, segments: usize) -> Self {
        let profile: Arc<dyn Profile> = Arc::from(profile::build(spec));
        let raw = profile.raw();
        let bundling = bundling_enabled(raw, profile.as_ref(), id);
        // A frame carries what its code fits. A fixed code is known now, so
        // the frames are cut to it — TM1280 fits half as much again as
        // TM2048, and a frame sized for TM2048 flies that much empty. An
        // adaptive channel may switch to the strongest code at any frame, so
        // it keeps the capacity every code can carry.
        let frame_capacity = match profile.selector() {
            Some(_) => FRAME_CAPACITY,
            None => LdpcPacketCoder::<RADIO_FRAME_SIZE>::max_payload_for(
                profile.adaptation().code(&TxHint::default()),
            ),
        };
        // A payload also has to fit one ctrl message, which is what bounds a
        // bonded channel on a light code: the frames it cannot fill are not
        // opened, so the payload stays a whole number of full frames.
        let segments = segments.min((CHANNEL_MAX_PAYLOAD / frame_capacity).max(1));
        let framed_mtu = frame_capacity * segments;
        let limits = ChannelLimits {
            mtu: if raw {
                RAW_MTU as u32
            } else if bundling {
                (framed_mtu - 3) as u32
            } else {
                framed_mtu as u32
            },
            frame_capacity: if raw {
                RAW_MTU as u32
            } else {
                frame_capacity as u32
            },
        };
        Self {
            id,
            profile,
            openers: HashMap::new(),
            queue: VecDeque::new(),
            // A bundle may span more than one frame, so a stream of small
            // packets does not go out one or two to a frame while the queue
            // overflows behind them — but not the whole MTU. A bundle is one
            // payload, and a payload is lost when any of its frames is: at
            // six frames the bundle dies to a single loss six times as often
            // as one frame does, and the real-time traffic that bundles is
            // exactly what cannot be resent. Two frames doubles the fill for
            // twice the exposure; past that the fill gain shrinks while the
            // exposure keeps compounding.
            bundler: Bundler::new(bundle_capacity(&limits)),
            limits,
            stats: ChannelStats::default(),
            burst: 0,
            rest_until: None,
        }
    }

    pub fn raw(&self) -> bool {
        self.profile.raw()
    }

    /// See [`bundling_enabled`].
    pub fn bundling(&self) -> bool {
        bundling_enabled(self.raw(), self.profile.as_ref(), self.id)
    }

    /// Clients receiving this channel's payloads.
    pub fn receivers(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        self.openers
            .iter()
            .filter(|(_, o)| o.rx)
            .map(|(addr, _)| *addr)
    }

    /// `true` when there is something to send and the channel is not in a
    /// listen gap.
    pub fn ready(&self, now: Instant) -> bool {
        let has_work = !self.queue.is_empty()
            || (self.bundling() && self.bundler.expired(self.profile.tdd().bundle_window, now));
        has_work && self.rest_until.map_or(true, |until| now >= until)
    }

    fn enqueue(&mut self, payload: Vec<u8>, hint: TxHint) -> ChannelSendResponse {
        if payload.is_empty() || payload.len() > self.limits.mtu as usize {
            return ChannelSendResponse::Oversize {
                max: self.limits.mtu,
            };
        }
        match self.profile.runtime().queue {
            QueuePolicy::Block { depth } if self.queue.len() >= usize::from(depth) => {
                return ChannelSendResponse::QueueFull;
            }
            QueuePolicy::DropOldest { depth } if self.queue.len() >= usize::from(depth) => {
                self.queue.pop_front();
                self.stats.tx_dropped += 1;
            }
            _ => {}
        }
        self.queue.push_back(Queued { payload, hint });
        self.stats.tx_payloads += 1;
        ChannelSendResponse::Accepted
    }
}

pub struct Table {
    /// The modules this table's channels run on: one for a plain table, all of
    /// them for the bonded table.
    modules: Vec<u8>,
    /// Frames a framed payload may span here; sets every channel's MTU.
    segments: usize,
    channels: HashMap<ChannelId, ChannelState>,
    raw: Option<ChannelId>,
}

pub type SharedTable = Arc<Mutex<Table>>;

impl Table {
    /// The channels bound to one module.
    pub fn new(module: u8) -> SharedTable {
        Arc::new(Mutex::new(Self {
            modules: vec![module],
            segments: SEGMENTS,
            channels: HashMap::new(),
            raw: None,
        }))
    }

    /// The channels that span every module at once. One payload's frames are
    /// spread across the radios, so throughput scales with them and a payload
    /// may be [`BONDED_SEGMENTS`] frames long instead of [`SEGMENTS`].
    pub fn bonded(modules: Vec<u8>) -> SharedTable {
        Arc::new(Mutex::new(Self {
            modules,
            segments: BONDED_SEGMENTS,
            channels: HashMap::new(),
            raw: None,
        }))
    }

    pub fn modules(&self) -> &[u8] {
        &self.modules
    }

    pub fn bonded_table(&self) -> bool {
        self.modules.len() > 1
    }

    pub fn segments(&self) -> usize {
        self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    pub fn get(&self, id: ChannelId) -> Option<&ChannelState> {
        self.channels.get(&id)
    }

    pub fn get_mut(&mut self, id: ChannelId) -> Option<&mut ChannelState> {
        self.channels.get_mut(&id)
    }

    /// The channel that takes frames which are not framed packets, if any.
    pub fn raw_channel(&self) -> Option<ChannelId> {
        self.raw
    }

    pub fn ids(&self) -> impl Iterator<Item = ChannelId> + '_ {
        self.channels.keys().copied()
    }

    /// Opens `id` for `opener`, creating the channel on first open. A second
    /// opener must ask for the same profile: the profile is the contract both
    /// ends of the protocol share, not a per-client preference.
    pub fn open(
        &mut self,
        id: ChannelId,
        spec: &ProfileSpec,
        opener: SocketAddr,
        tx: bool,
        rx: bool,
        now: Instant,
    ) -> Result<ChannelLimits, String> {
        let entry = Opener {
            tx,
            rx,
            last_seen: now,
        };
        if let Some(existing) = self.channels.get_mut(&id) {
            existing.openers.insert(opener, entry);
            return Ok(existing.limits);
        }

        let mut state = ChannelState::new(id, spec, self.segments);
        if state.raw() {
            if let Some(other) = self.raw {
                return Err(format!(
                    "module {:?} already has a raw channel ({})",
                    self.modules,
                    other.raw()
                ));
            }
            self.raw = Some(id);
        }
        let limits = state.limits;
        state.openers.insert(opener, entry);
        log::info!(
            "channel {} opened on module{} {:?} by {} ({}, mtu {})",
            id.raw(),
            if self.bonded_table() { "s" } else { "" },
            self.modules,
            opener,
            profile::describe(spec),
            limits.mtu
        );
        self.channels.insert(id, state);
        Ok(limits)
    }

    /// Removes `opener` from `id`; the channel goes when its last opener does.
    pub fn close(&mut self, id: ChannelId, opener: SocketAddr) {
        let gone = match self.channels.get_mut(&id) {
            Some(state) => {
                state.openers.remove(&opener);
                state.openers.is_empty()
            }
            None => false,
        };
        if gone {
            self.remove(id);
        }
    }

    fn remove(&mut self, id: ChannelId) {
        if self.channels.remove(&id).is_some() {
            if self.raw == Some(id) {
                self.raw = None;
            }
            log::info!("channel {} closed on module {:?}", id.raw(), self.modules);
        }
    }

    pub fn enqueue(&mut self, id: ChannelId, payload: Vec<u8>, hint: TxHint) -> ChannelSendResponse {
        match self.channels.get_mut(&id) {
            Some(state) => state.enqueue(payload, hint),
            None => ChannelSendResponse::Closed,
        }
    }

    /// Peer knowledge a client reports after parsing a payload.
    pub fn observe_peer(&mut self, id: ChannelId, peer: PeerKey, rssi: Option<i8>, class: Option<TrafficClass>) {
        let Some(state) = self.channels.get(&id) else {
            return;
        };
        let Some(selector) = state.profile.selector() else {
            return;
        };
        if let Some(rssi) = rssi {
            selector.observe_rssi(peer, rssi);
        }
        if let Some(class) = class {
            selector.set_class(peer, class);
        }
    }

    pub fn stats(&self, id: ChannelId) -> Option<ChannelStats> {
        self.channels.get(&id).map(|state| state.stats)
    }

    /// Any message from `addr` proves the client is alive.
    pub fn touch(&mut self, addr: SocketAddr, now: Instant) {
        for state in self.channels.values_mut() {
            if let Some(opener) = state.openers.get_mut(&addr) {
                opener.last_seen = now;
            }
        }
    }

    /// Drops openers not heard from within [`OPENER_TTL`] and the channels
    /// they were the last to hold. Returns the ids that closed.
    pub fn expire_openers(&mut self, now: Instant) -> Vec<ChannelId> {
        let mut closed = Vec::new();
        for state in self.channels.values_mut() {
            state
                .openers
                .retain(|_, opener| now.duration_since(opener.last_seen) < OPENER_TTL);
            if state.openers.is_empty() {
                closed.push(state.id);
            }
        }
        for id in &closed {
            self.remove(*id);
        }
        closed
    }
}

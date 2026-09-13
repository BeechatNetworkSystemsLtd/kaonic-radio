//! What a client tells the radio daemon when it opens a channel, and what the
//! daemon reports back. Everything here crosses the ctrl wire, so it is plain
//! data: the algorithms these describe run inside the daemon.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use kaonic_fec::{FecCode, PeerKey, TrafficClass, TxHint};

/// Logical channel a payload belongs to. It is carried in every frame so a
/// receiver can route the frame to the program that opened the same channel;
/// both ends of a protocol therefore use the same id, which is why it is
/// derived from the protocol's name rather than chosen by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChannelId(u16);

impl ChannelId {
    /// The channel every node transmits today; Reticulum traffic.
    pub const DEFAULT: ChannelId = ChannelId(0);

    /// Ids `1..=WELLKNOWN_MAX` are assigned by hand to what ships with the
    /// product; [`ChannelId::of`] never produces one of them.
    pub const WELLKNOWN_MAX: u16 = 255;

    /// Derives an id from a protocol name. Two nodes running the same protocol
    /// agree without exchanging anything; two unrelated names collide with
    /// probability 1 in 65 280.
    pub fn of(name: &str) -> ChannelId {
        // FNV-1a, folded to 16 bits, mapped past the well-known range.
        let mut hash: u32 = 0x811C_9DC5;
        for byte in name.as_bytes() {
            hash ^= u32::from(*byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        let folded = ((hash >> 16) ^ (hash & 0xFFFF)) as u16;
        let span = u16::MAX - Self::WELLKNOWN_MAX;
        ChannelId(Self::WELLKNOWN_MAX + 1 + folded % span)
    }

    /// A hand-assigned id in `1..=WELLKNOWN_MAX`.
    pub const fn wellknown(id: u16) -> ChannelId {
        assert!(id >= 1 && id <= Self::WELLKNOWN_MAX, "well-known ids are 1..=255");
        ChannelId(id)
    }

    /// The exact wire value, for a protocol that dictates one.
    pub const fn from_raw(id: u16) -> ChannelId {
        ChannelId(id)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// How a channel should use the air. The daemon holds one implementation per
/// variant; `Custom` composes the building blocks it already knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSpec {
    /// TM2048 on every frame, blocking queue, interactive priority: exactly
    /// what every node does today.
    Robust,
    /// Adaptive code per peer, long bursts, bundling; for transfers.
    Bulk { class: TrafficClass },
    /// A fixed light code, newest-wins queue, real-time priority, and a
    /// bundling window equal to the packet interval; for voice.
    Voice { code: FecCode, packet_interval: Duration },
    /// No framing and no coding: one payload is one PSDU. At most one Raw
    /// channel per module; it receives every frame that is not a framed packet.
    Raw,
    /// Any combination of the pieces the daemon implements.
    Custom {
        coding: CodingSpec,
        tdd: Tdd,
        runtime: Runtime,
        dispatch: DispatchSpec,
    },
}

impl Default for ProfileSpec {
    fn default() -> Self {
        ProfileSpec::Robust
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodingSpec {
    Fixed(FecCode),
    Adaptive { default: TrafficClass },
}

/// Which module carries a frame when a channel spans several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchSpec {
    /// Alternate; throughput scales with the number of modules.
    RoundRobin,
    /// Whichever module has the least queued.
    LeastQueued,
    /// Every frame on every module; the receiver keeps the first copy.
    Diversity,
}

/// Airtime discipline of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tdd {
    /// Frames sent back-to-back before the channel yields the air.
    pub burst_frames: u8,
    /// Silence after a burst so a peer's reply can get through.
    pub listen_gap: Duration,
    /// How long to wait for more payloads to fill one frame; zero disables
    /// bundling and every payload gets its own frame.
    pub bundle_window: Duration,
    /// Most payloads packed into one frame.
    pub bundle_max: u8,
    /// Slot the channel prefers to transmit in, for slotted access on a
    /// crowded channel. `None` is plain CSMA.
    pub slot_offset: Option<u16>,
}

impl Default for Tdd {
    fn default() -> Self {
        Self {
            burst_frames: 8,
            listen_gap: Duration::from_millis(12),
            bundle_window: Duration::ZERO,
            bundle_max: 1,
            slot_offset: None,
        }
    }
}

/// Queueing behaviour of a channel and its standing against the other
/// channels on the same module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runtime {
    pub queue: QueuePolicy,
    pub priority: Priority,
    /// How long a partially received packet waits for its remaining segments.
    pub reassembly_window: Duration,
    /// Packet ids remembered to drop duplicates in diversity mode.
    pub dedup_window: u16,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            queue: QueuePolicy::Block { depth: 32 },
            priority: Priority::Interactive,
            reassembly_window: Duration::from_millis(500),
            dedup_window: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueuePolicy {
    /// `send` waits for room; nothing is lost.
    Block { depth: u16 },
    /// The oldest queued payload is dropped to make room; nothing waits.
    DropOldest { depth: u16 },
}

/// Strict order in which a module drains its channels: every `Realtime`
/// payload before any `Interactive`, and `Bulk` only when the others are idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Realtime,
    Interactive,
    Bulk,
}

/// What the daemon knows about a received payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RxInfo {
    pub module: u8,
    pub rssi: i8,
    /// Code the frame was coded with; `None` for a Raw channel.
    pub code: Option<FecCode>,
    /// Packet id from the frame header; `None` for a Raw channel.
    pub packet_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelStats {
    pub tx_payloads: u64,
    pub tx_frames: u64,
    pub tx_busy: u64,
    pub tx_errors: u64,
    pub tx_dropped: u64,
    pub rx_payloads: u64,
    pub rx_frames: u64,
    pub rx_duplicates: u64,
    pub decode_clean: u64,
    pub decode_corrected: u64,
    pub decode_failed: u64,
    /// Frames that decoded but whose payload was given up on: a partial
    /// payload evicted for a newer one, or a segment no reassembly slot could
    /// take. Distinct from `decode_failed`, which is the air's doing; this is
    /// the daemon running out of room. Appended after the older fields, and
    /// defaulted, so a peer built before it still reads the record.
    #[serde(default)]
    pub rx_reassembly_dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropReason {
    QueueFull,
    Stale,
    Oversize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelEvent {
    Transmitted { module: u8, code: Option<FecCode>, bytes: u32 },
    Received { module: u8, rssi: i8, code: Option<FecCode> },
    DecodeFailed { module: u8, rssi: i8 },
    Dropped { reason: DropReason },
    /// The daemon restarted its radio; module configuration was reset.
    RadioRestarted,
    /// The daemon closed the channel (its last opener went away, or the radio
    /// is gone).
    Closed,
}

/// Limits the daemon reports when a channel is opened, so a client can size
/// its payloads without knowing the coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLimits {
    /// Largest single payload the channel accepts.
    pub mtu: u32,
    /// Payload bytes one radio frame carries under the channel's current code;
    /// multiples of this waste nothing on padding.
    pub frame_capacity: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_avoid_the_reserved_ranges_and_are_stable() {
        let a = ChannelId::of("kaonic-audio-ptt");
        let b = ChannelId::of("kaonic-audio-ptt");
        let c = ChannelId::of("kaonic-atak-bridge");
        assert_eq!(a, b);
        assert_ne!(a, c);
        for id in [a, c] {
            assert!(id.raw() > ChannelId::WELLKNOWN_MAX);
        }
        assert_eq!(ChannelId::DEFAULT.raw(), 0);
        assert_eq!(ChannelId::wellknown(7).raw(), 7);
    }

    #[test]
    fn spec_round_trips_through_msgpack() {
        let spec = ProfileSpec::Custom {
            coding: CodingSpec::Adaptive {
                default: TrafficClass::Auto,
            },
            tdd: Tdd::default(),
            runtime: Runtime::default(),
            dispatch: DispatchSpec::Diversity,
        };
        let bytes = rmp_serde::to_vec(&spec).unwrap();
        let back: ProfileSpec = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(back, spec);
    }
}

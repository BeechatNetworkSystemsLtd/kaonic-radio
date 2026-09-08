//! Runtime forward-error-correction adaptation for radio links.
//!
//! Nothing here is persisted: a sender picks a [`TrafficClass`] for a
//! destination at runtime and the selector turns that, plus the last RSSI
//! heard from it, into a [`FecCode`] per frame. The default is the
//! wire-compatible TM2048, so a node that never sets a class behaves exactly
//! like older firmware.
//!
//! The destination key is generic so this layer stays independent of whatever
//! addresses the transport above it uses.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use kaonic_net::coder::{CoderStats, FecCode};

/// Opaque identity of the station a frame is for or from. Transports map
/// their own addresses onto it (a Reticulum address hash fits exactly); a
/// real-time protocol can use a talker id. The adaptation layer only ever
/// compares keys.
pub type PeerKey = [u8; 16];

/// What the sender knows about a frame before it is coded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TxHint {
    /// Station the payload is addressed to, when the protocol knows it.
    pub peer: Option<PeerKey>,
    /// Overrides the peer's configured class for this frame only.
    pub class: Option<TrafficClass>,
    /// Payload length in bytes.
    pub len: usize,
}

/// How a received frame was recovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeOutcome {
    /// Systematic bytes satisfied the CRC; no correction was needed.
    Clean,
    /// The iterative decoder had to correct bit errors.
    Corrected,
    /// The frame could not be recovered.
    Failed,
}

/// How a transmit attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxOutcome {
    Sent,
    ChannelBusy,
    Error,
}

/// The seam a channel profile plugs a coding policy into: asked for a code
/// before every frame, told what happened after. Implementations are shared
/// between the transmit and receive paths of one channel.
pub trait Adaptation: Send + Sync {
    /// Code for the next frame described by `hint`.
    fn code(&self, hint: &TxHint) -> FecCode;

    /// A frame was received and decoded with `code` (as its header said).
    fn on_decoded(&self, code: FecCode, outcome: DecodeOutcome, rssi: i8) {
        let _ = (code, outcome, rssi);
    }

    /// A frame coded with `code` was handed to the radio.
    fn on_transmitted(&self, code: FecCode, outcome: TxOutcome) {
        let _ = (code, outcome);
    }
}

/// The same code for every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedCode(pub FecCode);

impl Adaptation for FixedCode {
    fn code(&self, _hint: &TxHint) -> FecCode {
        self.0
    }
}

impl Adaptation for FecSelector<PeerKey> {
    fn code(&self, hint: &TxHint) -> FecCode {
        match (hint.class, hint.peer) {
            (Some(class), _) => class_code(class, self, hint.peer),
            (None, Some(peer)) => self.select(peer),
            (None, None) => class_code(self.default_class(), self, None),
        }
    }
}

/// Resolves a class against the selector's link knowledge for `peer`.
fn class_code(class: TrafficClass, selector: &FecSelector<PeerKey>, peer: Option<PeerKey>) -> FecCode {
    match class {
        TrafficClass::Robust => FecCode::Tm2048,
        TrafficClass::Fast => FecCode::Tm1280,
        TrafficClass::Fastest => FecCode::None,
        TrafficClass::Fixed(code) => code,
        TrafficClass::Auto => match peer {
            Some(peer) => auto_code(selector.last_rssi(&peer).map(i16::from), None),
            None => FecCode::Tm2048,
        },
    }
}

/// RSSI samples older than this no longer influence code choice.
const RSSI_TTL: Duration = Duration::from_secs(300);
/// Hysteresis around the RSSI thresholds so a fluttering link does not
/// flip codes every frame.
const HYSTERESIS_DB: i16 = 4;
const STRONG_DBM: i16 = -60;
const GOOD_DBM: i16 = -75;

/// What the sender cares about for a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrafficClass {
    /// Maximum correction, wire-compatible with every node (TM2048).
    #[default]
    Robust,
    /// Pick the cheapest code the measured link supports.
    Auto,
    /// Rate 4/5: minimum airtime and decode cost; only for strong links.
    Fast,
    /// Uncoded payload (CRC only): lab / very short range.
    Fastest,
    /// A specific code, chosen by the caller.
    Fixed(FecCode),
}

struct Inner<K> {
    default: TrafficClass,
    per_destination: HashMap<K, TrafficClass>,
    rssi: HashMap<K, (i8, Instant)>,
    chosen: HashMap<K, FecCode>,
}

impl<K> Default for Inner<K> {
    fn default() -> Self {
        Self {
            default: TrafficClass::default(),
            per_destination: HashMap::new(),
            rssi: HashMap::new(),
            chosen: HashMap::new(),
        }
    }
}

/// Shared between the radio interface (which asks per frame) and the
/// applications (which set classes and are told link quality).
pub struct FecSelector<K> {
    inner: Mutex<Inner<K>>,
    verified_fast: AtomicU64,
    decoded_full: AtomicU64,
    failed: AtomicU64,
    header_failed: AtomicU64,
}

impl<K: Eq + Hash + Copy> Default for FecSelector<K> {
    fn default() -> Self {
        Self::new(TrafficClass::Robust)
    }
}

impl<K: Eq + Hash + Copy> FecSelector<K> {
    pub fn new(default: TrafficClass) -> Self {
        Self {
            inner: Mutex::new(Inner {
                default,
                ..Inner::default()
            }),
            verified_fast: AtomicU64::new(0),
            decoded_full: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            header_failed: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<K>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_default(&self, class: TrafficClass) {
        self.lock().default = class;
    }

    pub fn default_class(&self) -> TrafficClass {
        self.lock().default
    }

    /// Class for frames addressed to `destination` (a destination hash or a
    /// link id - whatever ends up in the packet's destination field).
    pub fn set_class(&self, destination: K, class: TrafficClass) {
        self.lock().per_destination.insert(destination, class);
    }

    pub fn clear_class(&self, destination: &K) {
        let mut inner = self.lock();
        inner.per_destination.remove(destination);
        inner.chosen.remove(destination);
    }

    /// Record the RSSI of a frame that carried a packet for `destination`.
    /// Link ids are the same in both directions, so this also measures the
    /// path we transmit on.
    pub fn observe_rssi(&self, destination: K, rssi: i8) {
        self.lock().rssi.insert(destination, (rssi, Instant::now()));
    }

    pub fn last_rssi(&self, destination: &K) -> Option<i8> {
        self.lock()
            .rssi
            .get(destination)
            .filter(|(_, at)| at.elapsed() < RSSI_TTL)
            .map(|(rssi, _)| *rssi)
    }

    /// Code for the next frame addressed to `destination`.
    pub fn select(&self, destination: K) -> FecCode {
        let mut inner = self.lock();
        let dest = destination;
        let class = inner
            .per_destination
            .get(&dest)
            .copied()
            .unwrap_or(inner.default);
        match class {
            TrafficClass::Robust => FecCode::Tm2048,
            TrafficClass::Fast => FecCode::Tm1280,
            TrafficClass::Fastest => FecCode::None,
            TrafficClass::Fixed(code) => code,
            TrafficClass::Auto => {
                let rssi = inner
                    .rssi
                    .get(&dest)
                    .filter(|(_, at)| at.elapsed() < RSSI_TTL)
                    .map(|(rssi, _)| i16::from(*rssi));
                let previous = inner.chosen.get(&dest).copied();
                let code = auto_code(rssi, previous);
                inner.chosen.insert(dest, code);
                code
            }
        }
    }

    pub fn record_stats(&self, stats: CoderStats) {
        self.verified_fast
            .store(stats.verified_fast, Ordering::Relaxed);
        self.decoded_full
            .store(stats.decoded_full, Ordering::Relaxed);
        self.failed.store(stats.failed, Ordering::Relaxed);
        self.header_failed
            .store(stats.header_failed, Ordering::Relaxed);
    }

    /// Receive-side decoder counters for this interface.
    pub fn stats(&self) -> CoderStats {
        CoderStats {
            verified_fast: self.verified_fast.load(Ordering::Relaxed),
            decoded_full: self.decoded_full.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            header_failed: self.header_failed.load(Ordering::Relaxed),
        }
    }
}

/// Threshold decision with hysteresis: a stronger code is only left once
/// the signal is clearly above the threshold, and only re-entered once it
/// is clearly below.
fn auto_code(rssi: Option<i16>, previous: Option<FecCode>) -> FecCode {
    let Some(rssi) = rssi else {
        return FecCode::Tm2048;
    };
    let up = |threshold: i16| rssi >= threshold + HYSTERESIS_DB;
    let down = |threshold: i16| rssi < threshold - HYSTERESIS_DB;
    match previous {
        Some(FecCode::Tm1280) => {
            if down(STRONG_DBM) {
                if down(GOOD_DBM) {
                    FecCode::Tm2048
                } else {
                    FecCode::Tm1536
                }
            } else {
                FecCode::Tm1280
            }
        }
        Some(FecCode::Tm1536) => {
            if up(STRONG_DBM) {
                FecCode::Tm1280
            } else if down(GOOD_DBM) {
                FecCode::Tm2048
            } else {
                FecCode::Tm1536
            }
        }
        _ => {
            if up(STRONG_DBM) {
                FecCode::Tm1280
            } else if up(GOOD_DBM) {
                FecCode::Tm1536
            } else {
                FecCode::Tm2048
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_uses_hysteresis() {
        assert_eq!(auto_code(None, None), FecCode::Tm2048);
        assert_eq!(auto_code(Some(-50), None), FecCode::Tm1280);
        assert_eq!(auto_code(Some(-70), None), FecCode::Tm1536);
        assert_eq!(auto_code(Some(-90), None), FecCode::Tm2048);
        // Slightly below the strong threshold keeps the fast code...
        assert_eq!(auto_code(Some(-62), Some(FecCode::Tm1280)), FecCode::Tm1280);
        // ...until it is clearly below.
        assert_eq!(auto_code(Some(-65), Some(FecCode::Tm1280)), FecCode::Tm1536);
        assert_eq!(auto_code(Some(-90), Some(FecCode::Tm1280)), FecCode::Tm2048);
        // Slightly above the strong threshold does not promote from Tm1536.
        assert_eq!(auto_code(Some(-58), Some(FecCode::Tm1536)), FecCode::Tm1536);
        assert_eq!(auto_code(Some(-55), Some(FecCode::Tm1536)), FecCode::Tm1280);
    }

    #[test]
    fn classes_map_to_codes_and_default_is_compatible() {
        let selector: FecSelector<[u8; 16]> = FecSelector::default();
        let dest = [0u8; 16];
        assert_eq!(selector.select(dest), FecCode::Tm2048);
        selector.set_class(dest, TrafficClass::Fast);
        assert_eq!(selector.select(dest), FecCode::Tm1280);
        selector.set_class(dest, TrafficClass::Auto);
        assert_eq!(selector.select(dest), FecCode::Tm2048);
        selector.observe_rssi(dest, -45);
        assert_eq!(selector.select(dest), FecCode::Tm1280);
        selector.clear_class(&dest);
        assert_eq!(selector.select(dest), FecCode::Tm2048);
    }

    #[test]
    fn adaptation_uses_hint_class_then_peer_state_then_default() {
        let selector: FecSelector<PeerKey> = FecSelector::default();
        let peer = [7u8; 16];
        let no_hint = TxHint::default();
        assert_eq!(selector.code(&no_hint), FecCode::Tm2048);

        selector.set_class(peer, TrafficClass::Fast);
        assert_eq!(selector.code(&TxHint { peer: Some(peer), ..TxHint::default() }), FecCode::Tm1280);

        // An explicit class on the hint wins over the peer's class.
        let hint = TxHint { peer: Some(peer), class: Some(TrafficClass::Fastest), len: 10 };
        assert_eq!(selector.code(&hint), FecCode::None);

        // Fixed policy ignores everything.
        assert_eq!(FixedCode(FecCode::Tc512).code(&hint), FecCode::Tc512);
    }

    #[test]
    fn per_destination_classes_are_independent() {
        let selector: FecSelector<u8> = FecSelector::default();
        selector.set_class(1, TrafficClass::Fastest);
        assert_eq!(selector.select(1), FecCode::None);
        assert_eq!(selector.select(2), FecCode::Tm2048);
    }
}

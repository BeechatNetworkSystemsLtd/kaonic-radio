//! One engine thread per channel table: drains channel queues onto the radios
//! and turns received frames back into payloads for the clients that opened the
//! channel. Coders live here, so LDPC work never runs on the real-time radio
//! worker, never on the daemon's async runtime, and never under the shared
//! table lock.
//!
//! A plain engine owns one module. The bonded engine owns every module and
//! spreads one payload's frames across them, so a transfer runs at the sum of
//! the radios' rates and a payload may span more frames than one radio's
//! channel allows. Reassembly needs no help for this: the muxer keys frames by
//! packet id and sequence number, never by the radio they arrived on, so the
//! far side puts a bonded payload back together with the code that was already
//! there.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kaonic_ctrl::channel::spec::{ChannelEvent, ChannelId, RxInfo, TxHint};
use kaonic_ctrl::protocol::{RADIO_FRAME_SIZE, ReceiveModule};
use kaonic_fec::{DecodeOutcome, TxOutcome};
use kaonic_frame::frame::{Frame, FrameSegment};
use kaonic_net::coder::{FecCode, LdpcPacketCoder};
use kaonic_net::network::{Network, Received};
use kaonic_radio::platform::PlatformRadioFrame;
use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc};

use kaonic_ctrl::channel::spec::DispatchSpec;

use super::bundler::Bundler;
use super::table::{ChannelState, SharedTable};
use crate::radio_worker::{RadioHandle, TxBatch};

/// Frames pulled from the radio per pass, so transmit never starves receive.
const RX_BATCH: usize = 16;
/// Payload shares allowed in flight on each radio at once. Two is enough to
/// cover one payload's encode with the previous payload's airtime; more would
/// only deepen the queue the TDD pacing already governs. The bound is per
/// radio rather than per payload so a radio that has finished its share is
/// not counted as busy on its sibling's account.
const TX_PIPELINE: usize = 2;
/// How far apart the radios' shares are started when both were idle: about
/// half a frame's airtime. Started together, their frames also end together,
/// and the far side — one core reading two chips over SPI — then has both
/// readouts due at once and loses whichever comes second. Once the radios are
/// busy their own rates keep them apart and no offset is needed.
const TX_STAGGER: Duration = Duration::from_millis(3);
/// Sleep when there is nothing to do.
const IDLE: Duration = Duration::from_millis(1);
const EXPIRE_EVERY: Duration = Duration::from_secs(1);

/// `SEG` is how many radio frames one framed payload may span and `SLOTS` how
/// many payloads may be in reassembly at once; the table the engine serves
/// decides both, and they are consts so the muxer's state stays a fixed-size
/// array.
type ChannelNet<const SEG: usize, const SLOTS: usize> =
    Network<RADIO_FRAME_SIZE, SEG, SLOTS, LdpcPacketCoder<RADIO_FRAME_SIZE>>;

/// A decoded payload on its way to one client.
pub struct Deliver {
    pub to: SocketAddr,
    pub id: ChannelId,
    pub payload: Arc<Vec<u8>>,
    pub info: RxInfo,
}

/// An event for every client of a channel.
pub struct Notify {
    pub to: SocketAddr,
    pub id: ChannelId,
    pub event: ChannelEvent,
}

/// Coder and reassembly state, owned by the engine thread.
struct Coding<const SEG: usize, const SLOTS: usize> {
    net: ChannelNet<SEG, SLOTS>,
}

impl<const SEG: usize, const SLOTS: usize> Coding<SEG, SLOTS> {
    /// The reassembly window comes from the channel's profile, not from a
    /// constant: it is how long the far side's frames may be spread out, which
    /// the link decides. A channel that closed under us keeps the default.
    fn for_channel(table: &SharedTable, id: ChannelId) -> Self {
        let shape = table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|s| (s.profile.runtime().reassembly_window, s.limits.frame_capacity as usize));
        let net = match shape {
            Some((window, segment)) => ChannelNet::new_sized(LdpcPacketCoder::new(), window, segment),
            None => ChannelNet::new(LdpcPacketCoder::new()),
        };
        Self { net }
    }
}

/// Monotonic milliseconds for the reassembly timeouts.
fn now_ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

/// Starts the engine for `table`. `modules` and `radios` are parallel: index
/// `i` of each is the same radio, and a bonded engine is simply one with more
/// than one entry.
pub fn spawn<const SEG: usize, const SLOTS: usize>(
    modules: Vec<u8>,
    radios: Vec<RadioHandle>,
    mut frames_in: broadcast::Receiver<Box<ReceiveModule>>,
    table: SharedTable,
    deliver: mpsc::Sender<Deliver>,
    notify: mpsc::Sender<Notify>,
) {
    assert_eq!(modules.len(), radios.len(), "one radio handle per module");
    assert!(!modules.is_empty(), "an engine needs at least one module");

    let name = modules
        .iter()
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join("+");

    std::thread::Builder::new()
        .name(format!("kaonic-channels-{name}"))
        .spawn(move || {
            let start = Instant::now();
            let mut coders: HashMap<ChannelId, Coding<SEG, SLOTS>> = HashMap::new();
            let mut assembly = FrameSegment::<RADIO_FRAME_SIZE, SEG>::new();
            // Header peeks reuse one coder: building a fresh one per frame
            // would zero its LDPC buffers on every frame the engine sees.
            let mut peek = LdpcPacketCoder::<RADIO_FRAME_SIZE>::new();
            // Scratch for the per-pass share count, reused rather than
            // reallocated: the loop spins while there is work to do.
            let mut outstanding = vec![0usize; radios.len()];
            let mut tx_frames = [Frame::<RADIO_FRAME_SIZE>::new(); SEG];
            let mut pending: Vec<PendingTx> = Vec::with_capacity(TX_PIPELINE * radios.len());
            // Where the next payload's round-robin starts. Carrying it across
            // payloads is what keeps a stream of one-frame payloads from all
            // landing on the first radio while the second sits idle.
            let mut next_radio = 0usize;
            let mut last_expire = Instant::now();

            loop {
                let now = Instant::now();
                let mut busy = false;

                if now.duration_since(last_expire) >= EXPIRE_EVERY {
                    last_expire = now;
                    let closed = table.lock().unwrap_or_else(|e| e.into_inner()).expire_openers(now);
                    for id in closed {
                        coders.remove(&id);
                    }
                }

                // ── retire transmits the radios have finished ───────────────
                // Shares are handed to the radios as they come due and polled
                // one radio at a time, so a payload is booked as soon as its
                // last share is acknowledged, whichever radio that was on.
                let mut i = 0;
                while i < pending.len() {
                    pending[i].submit_due(&radios, now);
                    match pending[i].poll() {
                        Some((sent, errors)) => {
                            let done = pending.remove(i);
                            busy = true;
                            finalize_tx(&table, &modules, &notify, &done, sent, errors);
                        }
                        None => i += 1,
                    }
                }

                // ── transmit: keep the air fed while the CPU encodes ────────
                // Submitting without waiting is what makes bonding pay: the
                // radios stay busy through the next payload's LDPC encode
                // instead of going quiet for it. The gate is per radio: a new
                // payload goes out once no radio holds `TX_PIPELINE` shares,
                // so the one that finished first is fed again without the
                // sibling having to catch up.
                outstanding_shares(&pending, &mut outstanding);
                if outstanding.iter().all(|&n| n < TX_PIPELINE) {
                    if let Some(job) = take_tx_job(&table, now) {
                    busy = true;
                    let frames: &[PlatformRadioFrame] = match &job.kind {
                        TxKind::Raw(payload) => {
                            tx_frames[0] = Frame::new_from_slice(payload);
                            &tx_frames[..1]
                        }
                        TxKind::Framed { payload, code } => {
                            let coding = coders
                                .entry(job.id)
                                .or_insert_with(|| Coding::for_channel(&table, job.id));
                            coding.net.coder_mut().set_tx_fec(*code);
                            coding.net.coder_mut().set_tx_channel(job.id.raw());
                            match coding.net.transmit(payload, OsRng, &mut tx_frames) {
                                Ok(frames) => {
                                    let count = frames.len();
                                    &tx_frames[..count]
                                }
                                Err(err) => {
                                    log::warn!("channel {}: encode failed: {err:?}", job.id.raw());
                                    &tx_frames[..0]
                                }
                            }
                        }
                    };

                    if !frames.is_empty() {
                        // Charge the burst now, not when the radios answer.
                        // With payloads in flight the acknowledgement comes
                        // too late to pace anything, and the channel would
                        // transmit straight through its own listen gap —
                        // which the far side cannot absorb, because the
                        // RF215 has one receive buffer and delivery falls off
                        // sharply as the frame rate climbs.
                        let queued = frames.len() as u8;
                        {
                            let mut guard =
                                table.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(state) = guard.get_mut(job.id) {
                                let tdd = state.profile.tdd();
                                state.burst = state.burst.saturating_add(queued);
                                if state.burst >= tdd.burst_frames {
                                    state.burst = 0;
                                    state.rest_until =
                                        Some(Instant::now() + tdd.listen_gap);
                                }
                            }
                        }
                        let idle = outstanding.iter().all(|&n| n == 0);
                        // The first frame goes to the radio with the least
                        // queued, the round-robin position deciding ties.
                        // Going by position alone, a stream of one-frame
                        // payloads kept handing frames to a radio that was
                        // already at the pipeline limit while its sibling
                        // ran dry: 18 ms a frame against 13.5 with the
                        // frames spread evenly.
                        let first = (0..radios.len())
                            .map(|k| (next_radio + k) % radios.len())
                            .min_by_key(|&r| outstanding[r])
                            .unwrap_or(next_radio);
                        let shares = dispatch(
                            radios.len(),
                            job.dispatch,
                            frames,
                            first,
                            idle,
                            Instant::now(),
                        );
                        next_radio = (first + frames.len()) % radios.len();
                        let mut tx = PendingTx {
                            id: job.id,
                            code: job.code,
                            shares,
                        };
                        tx.submit_due(&radios, Instant::now());
                        pending.push(tx);
                    }
                    }
                }

                // ── receive: route by header, decode only for the owner ─────
                for _ in 0..RX_BATCH {
                    let rx = match frames_in.try_recv() {
                        Ok(rx) => rx,
                        Err(broadcast::error::TryRecvError::Lagged(n)) => {
                            log::warn!("channels-{name}: receive lagged by {n} frame(s)");
                            continue;
                        }
                        Err(_) => break,
                    };
                    if !modules.contains(&(rx.module as u8)) {
                        continue;
                    }
                    // Peeking a header is an LDPC decode, and every engine sees
                    // every frame. On a single-core node that redundant work
                    // competes with the decode that actually matters, so an
                    // engine holding no channels drops out here rather than
                    // decoding a header it can only throw away.
                    if table.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                        continue;
                    }
                    busy = true;
                    handle_frame(
                        rx.module as u8,
                        &rx,
                        &table,
                        &mut coders,
                        &mut assembly,
                        &mut peek,
                        &deliver,
                        &notify,
                        now_ms(start),
                    );
                }

                if !busy {
                    if table.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                        coders.clear();
                    }
                    std::thread::park_timeout(IDLE);
                }
            }
        })
        .expect("spawn channel engine");
}

enum TxKind {
    Raw(Vec<u8>),
    Framed { payload: Vec<u8>, code: FecCode },
}

struct TxJob {
    id: ChannelId,
    code: FecCode,
    dispatch: DispatchSpec,
    kind: TxKind,
}

/// Spreads one payload's frames over the engine's radios. Nothing is handed
/// to a radio here; the shares carry the moment they may go, and
/// [`PendingTx::submit_due`] hands them over.
///
/// Every radio gets its share of the same payload, so on a bonded channel the
/// radios transmit at the same time and the payload takes roughly the airtime
/// of its longest share rather than the sum of them.
fn dispatch(
    radios: usize,
    spec: DispatchSpec,
    frames: &[PlatformRadioFrame],
    first: usize,
    idle: bool,
    now: Instant,
) -> Vec<Share> {
    // One radio: nothing to spread, and the frames stay in order on the air.
    if radios == 1 {
        return vec![Share::new(0, frames.to_vec(), now)];
    }

    // Round-robin from `first`: frame `i` goes to radio `(first + i) % n`, so
    // a payload of `n` frames or more keeps every radio busy. The receiver
    // reassembles by sequence number, so the order frames land in does not
    // matter.
    //
    // `LeastQueued` and `Diversity` are accepted by the spec but not
    // implemented; both fall back to round-robin rather than refusing a
    // channel that is otherwise fine.
    let _ = spec;

    let mut split: Vec<Vec<PlatformRadioFrame>> = vec![Vec::new(); radios];
    for (i, frame) in frames.iter().enumerate() {
        split[(first + i) % radios].push(*frame);
    }

    // With every radio idle the shares would start together and so end
    // together; the ones after the first are held back a little instead.
    let mut shares = Vec::with_capacity(radios);
    for k in 0..radios {
        let radio = (first + k) % radios;
        let frames = std::mem::take(&mut split[radio]);
        if frames.is_empty() {
            continue;
        }
        let stagger = if idle { TX_STAGGER * shares.len() as u32 } else { Duration::ZERO };
        shares.push(Share::new(radio, frames, now + stagger));
    }
    shares
}

/// One radio's part of a payload: waiting to go, on the air, or accounted
/// for.
struct Share {
    radio: usize,
    /// Empty once handed to the radio.
    frames: Vec<PlatformRadioFrame>,
    bytes: usize,
    /// Not handed over before then.
    not_before: Instant,
    batch: Option<TxBatch>,
    outcome: Option<(u32, u32)>,
}

impl Share {
    fn new(radio: usize, frames: Vec<PlatformRadioFrame>, not_before: Instant) -> Self {
        Self {
            radio,
            bytes: frames.iter().map(|f| f.len()).sum(),
            frames,
            not_before,
            batch: None,
            outcome: None,
        }
    }
}

/// A payload spread over the radios and not yet acknowledged. Holding one of
/// these is what lets the engine get on with the next payload's LDPC encode,
/// and with receiving, while the air is busy.
struct PendingTx {
    id: ChannelId,
    code: FecCode,
    shares: Vec<Share>,
}

impl PendingTx {
    /// Hands over every share whose time has come.
    fn submit_due(&mut self, radios: &[RadioHandle], now: Instant) {
        for share in self.shares.iter_mut() {
            if share.batch.is_none() && now >= share.not_before {
                let frames = std::mem::take(&mut share.frames);
                share.batch = Some(radios[share.radio].submit_batch(&frames));
            }
        }
    }

    /// `Some((sent, errors))` once every radio has accounted for its share.
    /// Each share is polled on its own, so a radio that finished is not asked
    /// again while its sibling is still on the air.
    fn poll(&mut self) -> Option<(u32, u32)> {
        for share in self.shares.iter_mut() {
            if share.outcome.is_none() {
                if let Some(batch) = share.batch.as_mut() {
                    share.outcome = batch.poll();
                }
            }
        }

        let mut sent = 0u32;
        let mut errors = 0u32;
        for share in &self.shares {
            let (s, e) = share.outcome?;
            sent += s;
            errors += e;
        }
        Some((sent, errors))
    }
}

/// Shares per radio that are waiting or on the air.
fn outstanding_shares(pending: &[PendingTx], counts: &mut [usize]) {
    counts.fill(0);
    for tx in pending {
        for share in tx.shares.iter().filter(|s| s.outcome.is_none()) {
            counts[share.radio] += 1;
        }
    }
}

/// Picks the channel to serve next and takes its payload out of the shared
/// table, so the lock is released before any coding or radio work.
fn take_tx_job(table: &SharedTable, now: Instant) -> Option<TxJob> {
    let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
    let id = pick(&guard, now)?;
    let state = guard.get_mut(id)?;

    if state.raw() {
        let dispatch = state.profile.dispatch();
        let queued = state.queue.pop_front()?;
        return Some(TxJob {
            id,
            code: FecCode::None,
            dispatch,
            kind: TxKind::Raw(queued.payload),
        });
    }

    let tdd = state.profile.tdd();
    let (payload, hint) = if state.bundling() {
        // Bundles are capped below the MTU; see `bundle_capacity` for why.
        let capacity = super::table::bundle_capacity(&state.limits);
        let blocked = fill_bundle(state, capacity, now);
        // A bundle goes when its window has passed, when it is full, or
        // when the next payload in the queue would not fit — it is then as
        // full as it is going to get, and waiting the window out only
        // delays it (a video stream of two 1.4 KB packets per bundle sat
        // out 20 ms per bundle that way). It does not go merely because
        // the queue behind it is empty: under a steady trickle of small
        // packets that sends every one in its own frame, which is the
        // exact waste the window exists to prevent. The window is the
        // latency the profile accepted.
        if !state.bundler.is_empty()
            && (blocked
                || state.bundler.expired(tdd.bundle_window, now)
                || state.bundler.count() >= tdd.bundle_max
                || !state.bundler.fits(1, capacity))
        {
            (state.bundler.take(), TxHint::default())
        } else {
            return None;
        }
    } else {
        let queued = state.queue.pop_front()?;
        (queued.payload, queued.hint)
    };

    let code = state.profile.adaptation().code(&TxHint {
        len: payload.len(),
        ..hint
    });
    Some(TxJob {
        id,
        code,
        dispatch: state.profile.dispatch(),
        kind: TxKind::Framed { payload, code },
    })
}

/// Moves queued payloads into the bundler while they fit. Returns `true`
/// when it stopped at a payload that did not fit, so the bundle cannot
/// grow further.
fn fill_bundle(state: &mut ChannelState, capacity: usize, now: Instant) -> bool {
    let max = state.profile.tdd().bundle_max;
    while state.bundler.count() < max {
        let Some(front) = state.queue.front() else { return false };
        if !state.bundler.fits(front.payload.len(), capacity) {
            if state.bundler.is_empty() {
                // Larger than a bundle on its own: it goes as its own bundle,
                // and there is nothing to wait for.
                let queued = state.queue.pop_front().unwrap();
                state.bundler.push(&queued.payload, now);
            }
            return true;
        }
        let queued = state.queue.pop_front().unwrap();
        state.bundler.push(&queued.payload, now);
    }
    false
}

/// Strict priority, then the channel that has waited longest.
fn pick(table: &super::table::Table, now: Instant) -> Option<ChannelId> {
    table
        .ids()
        .filter_map(|id| table.get(id).filter(|s| s.ready(now)).map(|s| (id, s)))
        .min_by_key(|(_, s)| (s.profile.runtime().priority, s.stats.tx_frames))
        .map(|(id, _)| id)
}

/// Counts an event and yields the running total only when it is worth
/// logging: the first occurrence, then every `every`-th. Keeps the per-frame
/// paths from flooding the log while still showing a fault is ongoing.
fn sampled(counter: &AtomicU64, every: u64) -> Option<u64> {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    (n == 1 || n % every == 0).then_some(n)
}

#[allow(clippy::too_many_arguments)]
fn handle_frame<const SEG: usize, const SLOTS: usize>(
    module: u8,
    rx: &ReceiveModule,
    table: &SharedTable,
    coders: &mut HashMap<ChannelId, Coding<SEG, SLOTS>>,
    assembly: &mut FrameSegment<RADIO_FRAME_SIZE, SEG>,
    peek: &mut LdpcPacketCoder<RADIO_FRAME_SIZE>,
    deliver: &mpsc::Sender<Deliver>,
    notify: &mpsc::Sender<Notify>,
    now_ms: u128,
) {
    let bytes = rx.frame.as_slice();

    // A scratch coder peeks headers; the decode happens in the owner's.
    let header = peek.peek_header(bytes);

    static HEADER_FAILED: AtomicU64 = AtomicU64::new(0);
    static UNROUTED: AtomicU64 = AtomicU64::new(0);

    // Everything the routing decision needs comes out of one lock: the table
    // is also taken by the ctrl handler, and this runs per received frame.
    let (id, receivers, raw, bundling): (ChannelId, Vec<SocketAddr>, bool, bool) = {
        let guard = table.lock().unwrap_or_else(|e| e.into_inner());
        let id = match &header {
            Some(header) => ChannelId::from_raw(header.channel()),
            None => match guard.raw_channel() {
                Some(id) => id,
                None => {
                    if let Some(n) = sampled(&HEADER_FAILED, 20) {
                        let head: Vec<String> =
                            bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
                        log::info!(
                            "channels-{module}: {n} frame(s) with an unreadable header dropped (rssi {}, len {}, head {})",
                            rx.rssi,
                            bytes.len(),
                            head.join(" ")
                        );
                    }
                    return;
                }
            },
        };
        let Some(state) = guard.get(id) else {
            if let Some(n) = sampled(&UNROUTED, 100) {
                log::info!("channels-{module}: {n} frame(s) for channels nobody opened (last: {})", id.raw());
            }
            return; // nobody opened it: dropped before any payload decode
        };
        (id, state.receivers().collect(), state.raw(), state.bundling())
    };

    if raw || header.is_none() {
        let payload = Arc::new(bytes.to_vec());
        let info = RxInfo {
            module,
            rssi: rx.rssi,
            code: None,
            packet_id: None,
        };
        count_rx(table, id, 1, 1, DecodeOutcome::Clean, None, rx.rssi, module, notify);
        for to in receivers {
            push_delivery(deliver, Deliver { to, id, payload: payload.clone(), info });
        }
        return;
    }

    let header = header.unwrap();
    let code = FecCode::from_id(header.fec());
    let coding = coders
        .entry(id)
        .or_insert_with(|| Coding::for_channel(table, id));
    let frame = Frame::<RADIO_FRAME_SIZE>::new_from_slice(bytes);

    let before = coding.net.coder().stats();
    let received = match coding.net.receive(now_ms, &frame) {
        Ok(received) => received,
        Err(err) => {
            // What a failed frame looked like, sampled: the error kind says
            // whether the bytes were wrong or short, and the lengths say
            // whether the read-out matched what the header promised.
            static FAILED: AtomicU64 = AtomicU64::new(0);
            if let Some(n) = sampled(&FAILED, 25) {
                let hdr_len = header.len();
                let expected = code.map(|c| c.encoded_len(usize::from(hdr_len))).unwrap_or(0);
                log::info!(
                    "channels-{module}: decode failed #{n}: {err:?}, frame {} B, header len {} (expects {} B coded), seq {}/{}, rssi {}",
                    bytes.len(),
                    hdr_len,
                    expected,
                    header.seq(),
                    header.seq_count(),
                    rx.rssi
                );
            }
            count_rx(table, id, 1, 0, DecodeOutcome::Failed, code, rx.rssi, module, notify);
            return;
        }
    };
    let after = coding.net.coder().stats();
    let outcome = if after.decoded_full > before.decoded_full {
        DecodeOutcome::Corrected
    } else {
        DecodeOutcome::Clean
    };
    // A frame that decoded but could not be kept is loss the air did not
    // cause, and it used to vanish without a trace; the counters are what
    // tell a full reassembly table apart from a bad link.
    match received {
        Received::Slotted => {}
        Received::Duplicate => {
            let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = guard.get_mut(id) {
                state.stats.rx_duplicates += 1;
            }
        }
        Received::Evicted | Received::Refused => {
            static DROPPED: AtomicU64 = AtomicU64::new(0);
            if let Some(n) = sampled(&DROPPED, 100) {
                log::warn!(
                    "channels-{module}: {n} frame(s) dropped in reassembly so far (last: {received:?} on channel {})",
                    id.raw()
                );
            }
            let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = guard.get_mut(id) {
                state.stats.rx_reassembly_dropped += 1;
            }
        }
    }

    let mut payloads = 0u64;
    while let Ok(packet) = coding.net.process(now_ms, assembly) {
        let info = RxInfo {
            module,
            rssi: rx.rssi,
            code,
            packet_id: Some(header.id()),
        };
        if bundling {
            for part in Bundler::split(packet.as_slice()) {
                payloads += 1;
                let payload = Arc::new(part.to_vec());
                for to in &receivers {
                    push_delivery(deliver, Deliver { to: *to, id, payload: payload.clone(), info });
                }
            }
        } else {
            payloads += 1;
            let payload = Arc::new(packet.as_slice().to_vec());
            for to in &receivers {
                push_delivery(deliver, Deliver { to: *to, id, payload: payload.clone(), info });
            }
        }
    }
    count_rx(table, id, 1, payloads, outcome, code, rx.rssi, module, notify);
}

/// Books a completed transmit: counters, coding feedback, TDD pacing, and one
/// `Transmitted` event per radio that carried part of the payload, so a client
/// watching a bonded channel sees the split rather than a single total.
fn finalize_tx(
    table: &SharedTable,
    modules: &[u8],
    notify: &mpsc::Sender<Notify>,
    done: &PendingTx,
    sent: u32,
    errors: u32,
) {
    let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = guard.get_mut(done.id) else {
        return;
    };

    state.stats.tx_frames += u64::from(sent);
    state.stats.tx_errors += u64::from(errors);
    state.profile.adaptation().on_transmitted(
        done.code,
        if errors == 0 {
            TxOutcome::Sent
        } else {
            TxOutcome::Error
        },
    );

    // Pacing was charged at submit time; completion only books the outcome.
    let code = if state.raw() { None } else { Some(done.code) };
    for share in &done.shares {
        if share.bytes == 0 {
            continue;
        }
        let event = ChannelEvent::Transmitted {
            module: modules[share.radio],
            code,
            bytes: share.bytes as u32,
        };
        for to in state.openers.keys().copied() {
            let _ = notify.try_send(Notify {
                to,
                id: done.id,
                event,
            });
        }
    }
}

/// Hands a payload to the delivery task. The engine must not block on the
/// ctrl link, so a full queue drops the payload — and says so, because a
/// silent drop here would look exactly like loss on the air.
fn push_delivery(deliver: &mpsc::Sender<Deliver>, item: Deliver) {
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    if let Err(err) = deliver.try_send(item) {
        if let Some(n) = sampled(&DROPPED, 100) {
            log::warn!("channel delivery queue full ({n} dropped so far): {err}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn count_rx(
    table: &SharedTable,
    id: ChannelId,
    frames: u64,
    payloads: u64,
    outcome: DecodeOutcome,
    code: Option<FecCode>,
    rssi: i8,
    module: u8,
    notify: &mpsc::Sender<Notify>,
) {
    let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = guard.get_mut(id) else { return };
    state.stats.rx_frames += frames;
    state.stats.rx_payloads += payloads;
    let event = match outcome {
        DecodeOutcome::Clean => {
            state.stats.decode_clean += 1;
            ChannelEvent::Received { module, rssi, code }
        }
        DecodeOutcome::Corrected => {
            state.stats.decode_corrected += 1;
            ChannelEvent::Received { module, rssi, code }
        }
        DecodeOutcome::Failed => {
            state.stats.decode_failed += 1;
            ChannelEvent::DecodeFailed { module, rssi }
        }
    };
    if let Some(code) = code {
        state.profile.adaptation().on_decoded(code, outcome, rssi);
    }
    if matches!(event, ChannelEvent::DecodeFailed { .. }) {
        for to in state.openers.keys().copied() {
            let _ = notify.try_send(Notify { to, id, event });
        }
    }
}

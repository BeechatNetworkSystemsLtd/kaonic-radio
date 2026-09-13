//! Dedicated per-radio worker thread.
//!
//! Each radio is owned by one OS thread; the async side talks to it through
//! command channels only. The worker drains received frames immediately when
//! the IRQ signal fires, which keeps the single hardware RX buffer free well
//! before the next back-to-back frame can overwrite it.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use kaonic_ctrl::protocol::{RadioFrame, ReceiveModule};
use kaonic_radio::{
    error::KaonicError,
    platform::{PlatformRadio, PlatformRadioFrame},
    radio::Radio,
};
use radio_common::{Accelerator, Antenna, Modulation, RadioBand, RadioConfig};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::radio_server::SharedModuleStats;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Gap between back-to-back transmits, so the peer can drain its single
/// hardware RX buffer. 4 ms measured best; scaling with frame length was worse.
const TX_PACING: Duration = Duration::from_millis(4);


/// Gap to leave before sending the next frame.
///
/// This is what protects the far side's single receive buffer: a frame that
/// arrives before the previous one has been read out overwrites it, and the
/// receiver sees a corrupt frame rather than a missing one. The right gap is
/// set by how fast the peer drains, not by this radio, so
/// `KAONIC_TX_PACING_MS` overrides it for measurement.
fn tx_pacing(_frame_len: usize) -> Duration {
    static PACING: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *PACING.get_or_init(|| {
        std::env::var("KAONIC_TX_PACING_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(TX_PACING)
    })
}
/// Fallback poll interval when idle; a radio interrupt or queued command wakes
/// the worker earlier.
const IDLE_WAIT: Duration = Duration::from_millis(20);
/// Wait for a frame to start arriving. Stays short so transmit is not starved;
/// the driver then allows the frame its own airtime.
const RX_DRAIN_TIMEOUT: Duration = Duration::from_millis(1);
/// Receptions one transmit gives way to before its refusal is reported as
/// the outcome. Every yield is one bounded receive, so this is what caps how
/// long a peer that never stops sending can hold a transmit back.
const TX_RX_YIELDS: usize = 8;


/// Platform-neutral wakeup for the worker: signalled by the GPIO event
/// thread on each radio IRQ and by command senders on each enqueue.
#[derive(Debug, Default)]
pub struct WorkerSignal {
    count: Mutex<u64>,
    condvar: Condvar,
}

impl WorkerSignal {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn notify(&self) {
        {
            let mut count = self.count.lock().unwrap();
            *count = count.wrapping_add(1);
        }

        self.condvar.notify_all();
    }

    fn wait(&self, prev: u64, timeout: Duration) -> u64 {
        let count = self.count.lock().unwrap();
        let (count, _) = self
            .condvar
            .wait_timeout_while(count, timeout, |c| *c == prev)
            .unwrap();

        *count
    }
}

enum RadioCommand {
    Transmit(Box<PlatformRadioFrame>),
    SetConfig(RadioConfig),
    GetConfig,
    SetModulation(Modulation),
    GetModulation,
    SetAccelerator(Accelerator),
    GetAccelerator,
    SetAntenna(RadioBand, Antenna),
    GetAntenna(RadioBand),
}

enum RadioReply {
    Done(Result<(), KaonicError>),
    Config(RadioConfig),
    Modulation(Modulation),
    Accelerator(Accelerator),
    Antenna(Antenna),
}

struct RadioRequest {
    command: RadioCommand,
    reply: mpsc::Sender<RadioReply>,
}

/// Frames handed to a worker and not yet acknowledged; [`TxBatch::wait`]
/// collects the outcomes.
pub struct TxBatch {
    pending: Vec<mpsc::Receiver<RadioReply>>,
    errors: u32,
    sent: u32,
    started: Instant,
}

impl TxBatch {
    /// Blocks until every queued frame is transmitted or times out.
    /// Returns (sent, errors).
    pub fn wait(self) -> (u32, u32) {
        let mut sent = 0u32;
        let mut errors = self.errors;

        for reply_rx in self.pending {
            match reply_rx.recv_timeout(COMMAND_TIMEOUT) {
                Ok(RadioReply::Done(Ok(()))) => sent += 1,
                _ => errors += 1,
            }
        }

        (sent, errors)
    }

    /// Collects whatever replies have arrived without blocking, and reports
    /// the outcome once every frame is accounted for. This is what lets the
    /// engine encode the next payload while the radios are still busy with
    /// this one, instead of standing idle waiting on them.
    pub fn poll(&mut self) -> Option<(u32, u32)> {
        let mut sent = 0u32;
        let mut errors = 0u32;

        self.pending.retain(|reply_rx| match reply_rx.try_recv() {
            Ok(RadioReply::Done(Ok(()))) => {
                sent += 1;
                false
            }
            Ok(_) => {
                errors += 1;
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                errors += 1;
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
        });

        self.sent += sent;
        self.errors += errors;

        // A worker that never answers must not wedge the pipeline.
        if !self.pending.is_empty() && self.started.elapsed() >= COMMAND_TIMEOUT {
            self.errors += self.pending.len() as u32;
            self.pending.clear();
        }

        if self.pending.is_empty() {
            Some((self.sent, self.errors))
        } else {
            None
        }
    }
}

/// Handle for sending commands to a radio worker thread. Cloneable; every
/// call blocks the calling OS thread until the worker replies, so async
/// callers should wrap calls in `spawn_blocking`.
#[derive(Clone)]
pub struct RadioHandle {
    cmd_tx: mpsc::Sender<RadioRequest>,
    signal: Arc<WorkerSignal>,
}

impl RadioHandle {
    fn call(&self, command: RadioCommand) -> Result<RadioReply, KaonicError> {
        let (reply_tx, reply_rx) = mpsc::channel();

        self.cmd_tx
            .send(RadioRequest {
                command,
                reply: reply_tx,
            })
            .map_err(|_| KaonicError::HardwareError)?;
        self.signal.notify();

        reply_rx
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|_| KaonicError::Timeout)
    }

    fn call_done(&self, command: RadioCommand) -> Result<(), KaonicError> {
        match self.call(command)? {
            RadioReply::Done(result) => result,
            _ => Err(KaonicError::InvalidState),
        }
    }

    pub fn transmit(&self, frame: PlatformRadioFrame) -> Result<(), KaonicError> {
        self.call_done(RadioCommand::Transmit(Box::new(frame)))
    }

    /// Queues `frames` on the worker and returns without waiting. Splitting
    /// submit from wait is what lets a bonded channel keep several radios busy
    /// at once: every radio gets its frames before any of them is waited on.
    pub fn submit_batch(&self, frames: &[PlatformRadioFrame]) -> TxBatch {
        let mut pending = Vec::with_capacity(frames.len());
        let mut errors = 0u32;

        for frame in frames {
            let (reply_tx, reply_rx) = mpsc::channel();

            match self.cmd_tx.send(RadioRequest {
                command: RadioCommand::Transmit(Box::new(*frame)),
                reply: reply_tx,
            }) {
                Ok(_) => pending.push(reply_rx),
                Err(_) => errors += 1,
            }
        }

        self.signal.notify();

        TxBatch {
            pending,
            errors,
            sent: 0,
            started: Instant::now(),
        }
    }

    /// Queues all frames before collecting replies, so the worker transmits
    /// them back-to-back with no queue-empty gaps. Returns (sent, errors).
    pub fn transmit_batch(&self, frames: &[PlatformRadioFrame]) -> (u32, u32) {
        self.submit_batch(frames).wait()
    }

    pub fn set_config(&self, config: RadioConfig) -> Result<(), KaonicError> {
        self.call_done(RadioCommand::SetConfig(config))
    }

    pub fn get_config(&self) -> Result<RadioConfig, KaonicError> {
        match self.call(RadioCommand::GetConfig)? {
            RadioReply::Config(config) => Ok(config),
            _ => Err(KaonicError::InvalidState),
        }
    }

    pub fn set_modulation(&self, modulation: Modulation) -> Result<(), KaonicError> {
        self.call_done(RadioCommand::SetModulation(modulation))
    }

    pub fn get_modulation(&self) -> Result<Modulation, KaonicError> {
        match self.call(RadioCommand::GetModulation)? {
            RadioReply::Modulation(modulation) => Ok(modulation),
            _ => Err(KaonicError::InvalidState),
        }
    }

    pub fn set_accelerator(&self, accelerator: Accelerator) -> Result<(), KaonicError> {
        self.call_done(RadioCommand::SetAccelerator(accelerator))
    }

    pub fn get_accelerator(&self) -> Result<Accelerator, KaonicError> {
        match self.call(RadioCommand::GetAccelerator)? {
            RadioReply::Accelerator(accelerator) => Ok(accelerator),
            _ => Err(KaonicError::InvalidState),
        }
    }

    pub fn set_antenna(&self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        self.call_done(RadioCommand::SetAntenna(band, antenna))
    }

    pub fn get_antenna(&self, band: RadioBand) -> Result<Antenna, KaonicError> {
        match self.call(RadioCommand::GetAntenna(band))? {
            RadioReply::Antenna(antenna) => Ok(antenna),
            _ => Err(KaonicError::InvalidState),
        }
    }
}

pub fn spawn(
    module: usize,
    mut radio: PlatformRadio,
    signal: Arc<WorkerSignal>,
    rx_send: broadcast::Sender<Box<ReceiveModule>>,
    stats: SharedModuleStats,
) -> RadioHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<RadioRequest>();
    let handle = RadioHandle {
        cmd_tx,
        signal: signal.clone(),
    };

    std::thread::Builder::new()
        .name(format!("kaonic-radio-{}", module))
        .spawn(move || {
            // Below the GPIO event thread (10), above normal threads: RX
            // drains must not wait out an EEVDF timeslice.
            if !crate::radio_server::set_realtime_priority(9) {
                log::warn!("radio[{}] worker: can't set SCHED_FIFO priority", module);
            }

            let mut rx_frame = PlatformRadioFrame::new();
            let mut prev = 0u64;
            let (mut tx_exec_us, mut tx_gap_us, mut tx_frames) = (0u64, 0u64, 0u64);
            let mut last_tx_end: Option<Instant> = None;

            loop {
                let mut last_was_transmit = false;

                loop {
                    match cmd_rx.try_recv() {
                        Ok(request) => {
                            let tx_len = match &request.command {
                                RadioCommand::Transmit(frame) => Some(frame.as_slice().len()),
                                _ => None,
                            };

                            if let (Some(len), true) = (tx_len, last_was_transmit) {
                                let gap = tx_pacing(len);
                                if !gap.is_zero() {
                                    std::thread::sleep(gap);
                                }
                            }

                            let exec_start = Instant::now();
                            let reply = match request.command {
                                RadioCommand::Transmit(frame) => transmit_one(
                                    module,
                                    &mut radio,
                                    &frame,
                                    &mut rx_frame,
                                    &rx_send,
                                    &stats,
                                ),
                                command => execute(&mut radio, command),
                            };
                            let exec_us = exec_start.elapsed().as_micros() as u64;
                            let _ = request.reply.send(reply);

                            // Time inside the radio call versus everything
                            // else the worker does between frames.
                            if tx_len.is_some() {
                                tx_exec_us += exec_us;
                                tx_gap_us += last_tx_end
                                    .map(|end: Instant| end.elapsed().as_micros() as u64)
                                    .unwrap_or(0)
                                    .saturating_sub(exec_us);
                                tx_frames += 1;

                                if tx_frames % 100 == 0 {
                                    log::info!(
                                        "radio[{}] tx slot: {} us in radio, {} us elsewhere ({} frames)",
                                        module,
                                        tx_exec_us / tx_frames,
                                        tx_gap_us / tx_frames,
                                        tx_frames,
                                    );
                                }

                                last_tx_end = Some(Instant::now());
                            }

                            last_was_transmit = tx_len.is_some();
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return,
                    }
                }

                if drain_one(module, &mut radio, &mut rx_frame, &rx_send, &stats, RX_DRAIN_TIMEOUT) {
                    // A frame arrived; re-check commands before draining more.
                    continue;
                }

                prev = signal.wait(prev, IDLE_WAIT);

                let _ = radio.update_event();
            }
        })
        .expect("spawn radio worker");

    handle
}

/// Transmits one frame, receiving first whatever the radio already holds.
///
/// The radio is half-duplex with a single receive buffer: a transmit while a
/// frame is arriving cuts that frame off, and one while a finished frame is
/// still unread lets the next reception overwrite it. Either used to happen
/// whenever the queue held several frames, because the worker only received
/// once the queue was empty. The driver now refuses both, and the worker
/// asks first — one interrupt read — so the usual case never reaches the
/// refusal; a frame that starts in between does, and comes back here to be
/// received before the retry.
fn transmit_one(
    module: usize,
    radio: &mut PlatformRadio,
    frame: &PlatformRadioFrame,
    rx_frame: &mut PlatformRadioFrame,
    rx_send: &broadcast::Sender<Box<ReceiveModule>>,
    stats: &SharedModuleStats,
) -> RadioReply {
    let frame_len = frame.as_slice().len() as u64;
    let mut yields = 0usize;

    // The frame goes into the radio first, so the write is done before the
    // air is claimed. Once is enough: a drain in between reads the other
    // buffer.
    let _ = radio.stage(frame);

    let result = loop {
        if radio.receive_pending().unwrap_or(false) {
            drain_one(module, radio, rx_frame, rx_send, stats, RX_DRAIN_TIMEOUT);
        }

        let result = radio.transmit(frame);

        if !matches!(result, Err(KaonicError::RxPending)) {
            break result;
        }

        // Past the bound the refusal stands as the outcome: a transmit is
        // never allowed to cost the radio a frame it is receiving, so the
        // frame is reported lost rather than sent over one.
        if yields >= TX_RX_YIELDS {
            log::debug!(
                "radio[{}] tx gave way to {} receptions and was not sent",
                module,
                yields,
            );
            break result;
        }

        yields += 1;
    };

    match &result {
        Ok(_) => {
            stats.tx_packets.fetch_add(1, Ordering::Relaxed);
            stats.tx_bytes.fetch_add(frame_len, Ordering::Relaxed);
        }
        Err(_) => {
            stats.tx_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    RadioReply::Done(result)
}

fn execute(radio: &mut PlatformRadio, command: RadioCommand) -> RadioReply {
    match command {
        // Handled by `transmit_one`, which also needs the receive side.
        RadioCommand::Transmit(_) => RadioReply::Done(Err(KaonicError::InvalidState)),
        RadioCommand::SetConfig(config) => RadioReply::Done(radio.set_config(&config)),
        RadioCommand::GetConfig => RadioReply::Config(radio.get_config()),
        RadioCommand::SetModulation(modulation) => {
            RadioReply::Done(radio.set_modulation(&modulation))
        }
        RadioCommand::GetModulation => RadioReply::Modulation(radio.get_modulation()),
        RadioCommand::SetAccelerator(accelerator) => {
            RadioReply::Done(radio.set_accelerator(&accelerator))
        }
        RadioCommand::GetAccelerator => RadioReply::Accelerator(radio.get_accelerator()),
        RadioCommand::SetAntenna(band, antenna) => {
            RadioReply::Done(radio.set_antenna(band, antenna))
        }
        RadioCommand::GetAntenna(band) => RadioReply::Antenna(radio.get_antenna(band)),
    }
}

thread_local! {
    /// (radio time, publish time, frames) accumulated for the periodic report.
    static RX_STATS: std::cell::RefCell<(u64, u64, u64)> = std::cell::RefCell::new((0, 0, 0));
}

fn drain_one(
    module: usize,
    radio: &mut PlatformRadio,
    rx_frame: &mut PlatformRadioFrame,
    rx_send: &broadcast::Sender<Box<ReceiveModule>>,
    stats: &SharedModuleStats,
    timeout: Duration,
) -> bool {
    let recv_start = Instant::now();

    match radio.receive(rx_frame.clear(), timeout) {
        Ok(rr) => {
            let recv_us = recv_start.elapsed().as_micros() as u64;

            stats.rx_packets.fetch_add(1, Ordering::Relaxed);
            stats
                .rx_bytes
                .fetch_add(rx_frame.len() as u64, Ordering::Relaxed);

            let publish_start = Instant::now();

            let mut receive_module = Box::new(ReceiveModule::new());
            receive_module.module = module;
            receive_module.frame = RadioFrame::new_from_frame(rx_frame);
            receive_module.rssi = rr.rssi;

            let _ = rx_send.send(receive_module);

            // Radio call versus hand-off to clients; the hand-off queues a
            // fixed 2 KB structure whatever the frame length.
            RX_STATS.with(|cell| {
                let mut stats = cell.borrow_mut();
                stats.0 += recv_us;
                stats.1 += publish_start.elapsed().as_micros() as u64;
                stats.2 += 1;

                if stats.2 % 100 == 0 {
                    log::info!(
                        "radio[{}] rx slot: {} us in radio, {} us publishing ({} frames)",
                        module,
                        stats.0 / stats.2,
                        stats.1 / stats.2,
                        stats.2,
                    );
                }
            });

            true
        }
        Err(KaonicError::Timeout) => false,
        Err(e) => {
            stats.rx_errors.fetch_add(1, Ordering::Relaxed);
            log::warn!("radio[{}] receive error: {:?}", module, e);
            false
        }
    }
}

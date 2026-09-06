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
use tokio::sync::broadcast;

use crate::radio_server::SharedModuleStats;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Gap between back-to-back transmits: the peer needs ~3ms after each frame
/// to drain its single hardware RX buffer before the next frame overwrites it.
const TX_PACING: Duration = Duration::from_millis(4);
const IDLE_WAIT: Duration = Duration::from_millis(20);
const RX_DRAIN_TIMEOUT: Duration = Duration::from_millis(1);

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

    /// Queues all frames before collecting replies, so the worker transmits
    /// them back-to-back with no queue-empty gaps. Returns (sent, errors).
    pub fn transmit_batch(&self, frames: Vec<PlatformRadioFrame>) -> (u32, u32) {
        let mut pending = Vec::with_capacity(frames.len());
        let mut errors = 0u32;

        for frame in frames {
            let (reply_tx, reply_rx) = mpsc::channel();

            match self.cmd_tx.send(RadioRequest {
                command: RadioCommand::Transmit(Box::new(frame)),
                reply: reply_tx,
            }) {
                Ok(_) => pending.push(reply_rx),
                Err(_) => errors += 1,
            }
        }

        self.signal.notify();

        let mut sent = 0u32;

        for reply_rx in pending {
            match reply_rx.recv_timeout(COMMAND_TIMEOUT) {
                Ok(RadioReply::Done(Ok(()))) => sent += 1,
                _ => errors += 1,
            }
        }

        (sent, errors)
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

            loop {
                let mut last_was_transmit = false;

                loop {
                    match cmd_rx.try_recv() {
                        Ok(request) => {
                            let is_transmit =
                                matches!(request.command, RadioCommand::Transmit(_));

                            if is_transmit && last_was_transmit {
                                std::thread::sleep(TX_PACING);
                            }

                            let reply = execute(&mut radio, request.command, &stats);
                            let _ = request.reply.send(reply);

                            last_was_transmit = is_transmit;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return,
                    }
                }

                if drain_one(module, &mut radio, &mut rx_frame, &rx_send, &stats) {
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

fn execute(radio: &mut PlatformRadio, command: RadioCommand, stats: &SharedModuleStats) -> RadioReply {
    match command {
        RadioCommand::Transmit(frame) => {
            let frame_len = frame.as_slice().len() as u64;
            let result = radio.transmit(&frame);

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

fn drain_one(
    module: usize,
    radio: &mut PlatformRadio,
    rx_frame: &mut PlatformRadioFrame,
    rx_send: &broadcast::Sender<Box<ReceiveModule>>,
    stats: &SharedModuleStats,
) -> bool {
    match radio.receive(rx_frame.clear(), RX_DRAIN_TIMEOUT) {
        Ok(rr) => {
            stats.rx_packets.fetch_add(1, Ordering::Relaxed);
            stats
                .rx_bytes
                .fetch_add(rx_frame.len() as u64, Ordering::Relaxed);

            let mut receive_module = Box::new(ReceiveModule::new());
            receive_module.module = module;
            receive_module.frame = RadioFrame::new_from_frame(rx_frame);
            receive_module.rssi = rr.rssi;

            let _ = rx_send.send(receive_module);

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

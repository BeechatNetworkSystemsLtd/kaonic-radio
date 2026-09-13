use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use kaonic_frame::frame::Frame;
use radio_common::{
    frequency::BandwidthFilter, modulation::OfdmModulation, Hertz, Modulation, RadioConfig,
    RadioConfigBuilder,
};
use radio_rf215::{
    baseband::BasebandFrame,
    bus::{BusInterrupt, SpiBus},
    ChipMode, Rf215,
};

use crate::{
    error::KaonicError,
    platform::{
        air,
        kaonic1s::{fpga::Kaonic1SFpga, machine::create_radios},
        linux::{
            LinuxClock, LinuxGpioInterrupt, LinuxGpioReset, LinuxOutputPin, LinuxSpi, SharedBus,
        },
        linux_rf215::{AtomicInterrupt, IrqSignal},
    },
    radio::{Accelerator, Antenna, Radio, RadioBand, ReceiveResult, ScanResult},
};

pub mod fpga;
mod machine;

pub const FRAME_SIZE: usize = 2048usize;

pub type Kaonic1SBus = SpiBus<LinuxSpi, AtomicInterrupt, LinuxClock, LinuxGpioReset>;

#[derive(Debug)]
pub struct Kaonic1SRadioFem {
    /// Which radio this front-end belongs to; only used to scope the
    /// bring-up overrides to one of them.
    index: usize,
    flt_v1: LinuxOutputPin,
    flt_v2: LinuxOutputPin,
    flt_24: Option<LinuxOutputPin>,
    ant_24: Option<LinuxOutputPin>,
    ant_24_sel: Antenna,
}

impl Kaonic1SRadioFem {
    pub fn new(
        index: usize,
        flt_v1: LinuxOutputPin,
        flt_v2: LinuxOutputPin,
        flt_24: Option<LinuxOutputPin>,
        ant_24: Option<LinuxOutputPin>,
    ) -> Self {
        Self {
            index,
            flt_v1,
            flt_v2,
            flt_24,
            ant_24,
            ant_24_sel: Antenna::default(),
        }
    }

    pub fn set_antenna(&mut self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        match band {
            RadioBand::Band24 => {
                if self.ant_24.is_none() && antenna != Antenna::Internal {
                    return Err(KaonicError::NotSupported);
                }

                self.ant_24_sel = antenna;
                self.apply_ant_24()
            }
            // The sub-GHz path has no antenna switch on this board.
            RadioBand::Band09 => match antenna {
                Antenna::Internal => Ok(()),
                Antenna::External => Err(KaonicError::NotSupported),
            },
        }
    }

    pub fn antenna(&self, band: RadioBand) -> Antenna {
        match band {
            RadioBand::Band24 => self.ant_24_sel,
            RadioBand::Band09 => Antenna::Internal,
        }
    }

    fn apply_ant_24(&mut self) -> Result<(), KaonicError> {
        if let Some(ant_24) = &mut self.ant_24 {
            match self.ant_24_sel {
                Antenna::Internal => ant_24.set_low()?,
                Antenna::External => ant_24.set_high()?,
            }
        }

        Ok(())
    }

    fn set_bandwidth_filter(
        &mut self,
        filter: BandwidthFilter,
        freq: Hertz,
    ) -> Result<(), KaonicError> {
        let freq = freq.as_mhz();

        match filter {
            BandwidthFilter::Narrow => {
                log::debug!("set narrowband filter");

                if (902 <= freq) && (freq <= 928) {
                    self.flt_v1.set_high()?;
                    self.flt_v2.set_low()?;
                }

                if (862 <= freq) && (freq <= 876) {
                    self.flt_v1.set_low()?;
                    self.flt_v2.set_high()?;
                }

                // Use Wideband filter
                if freq < 862 {
                    log::trace!(
                        "narrow band is not supported for {}MHz, wideband will be used",
                        freq
                    );

                    self.flt_v1.set_high()?;
                    self.flt_v2.set_low()?;
                }
            }
            BandwidthFilter::Wide => {
                log::debug!("set wideband filter");
                self.flt_v1.set_high()?;
                self.flt_v2.set_low()?;
            }
        }

        // Bring-up override: force this radio's sub-GHz filter select, for a
        // board whose filter bank is wired differently from the first radio's.
        // `KAONIC_RF09_FLT_1=10` means v1 high, v2 low on radio 1.
        if let Ok(sel) = std::env::var(format!("KAONIC_RF09_FLT_{}", self.index)) {
            let bytes = sel.as_bytes();
            if bytes.len() == 2 {
                log::info!("radio[{}] rf09 filter override: {}", self.index, sel);
                if bytes[0] == b'1' {
                    self.flt_v1.set_high()?;
                } else {
                    self.flt_v1.set_low()?;
                }
                if bytes[1] == b'1' {
                    self.flt_v2.set_high()?;
                } else {
                    self.flt_v2.set_low()?;
                }
            }
        }

        Ok(())
    }

    pub fn adjust(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        self.apply_ant_24()?;

        self.set_bandwidth_filter(config.bandwidth_filter, config.freq)?;

        // NOTE: Should be set to 0
        if let Some(flt_24) = &mut self.flt_24 {
            let _ = flt_24.set_low();
        }

        Ok(())
    }
}

pub type Kaonic1SFrame = Frame<FRAME_SIZE>;
pub type Kaonic1SRf215 = Rf215<SharedBus<Kaonic1SBus>>;

#[derive(Debug)]
pub struct Kaonic1SRadioEvent {
    signal: Arc<IrqSignal>,
    irq: LinuxGpioInterrupt,
}

impl Kaonic1SRadioEvent {
    pub fn new(signal: Arc<IrqSignal>, irq: LinuxGpioInterrupt) -> Self {
        Self { signal, irq }
    }

    pub fn wait_for_event(&mut self, timeout: Option<core::time::Duration>) -> bool {
        if self.irq.wait_on_interrupt(timeout) {
            self.signal.notify();

            return true;
        }

        false
    }
}

pub struct Kaonic1SRadio {
    fem: Kaonic1SRadioFem,
    radio: Kaonic1SRf215,
    event: Arc<Mutex<Kaonic1SRadioEvent>>,
    bb_frame: BasebandFrame,
    config: RadioConfig,
    modulation: Modulation,
    accelerator: Accelerator,
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,

    noise_dbm: i8,

    /// The transmit buffer holds the frame `stage` was given.
    staged: bool,
    /// Which of the node's radios this is; the air gate is keyed by it.
    index: usize,

    last_rx_arm: Instant,
    last_peer_heard: Instant,
    peer_dbm: i8,
    backoff_window: u32,
    random_state: u64,
    stats: Kaonic1SRadioStats,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Kaonic1SRadioStats {
    /// Transmissions deferred because the channel was busy.
    pub cca_busy: u64,
    /// Times the receiver was re-armed after going silent.
    pub rx_rearms: u64,
    /// Frames transmitted, and the time they spent inside the driver.
    pub tx_count: u64,
    pub tx_time_us: u64,
}

impl Kaonic1SRadioStats {
    /// Mean time one frame spent in the driver.
    pub fn tx_micros_per_frame(&self) -> u64 {
        self.tx_time_us / self.tx_count.max(1)
    }
}

/// Transmit attempts before giving up, including channel-busy retries.
const TX_ATTEMPTS: usize = 8;
/// Backoff slot: about one short frame plus turnaround at the fastest rate.
const CCA_BACKOFF_SLOT: core::time::Duration = core::time::Duration::from_micros(500);
const CCA_BACKOFF_WINDOW_MIN: u32 = 4;
const CCA_BACKOFF_WINDOW_MAX: u32 = 64;
/// Headroom over the measured noise floor before the channel counts as busy.
/// How far below the weakest neighbour heard the channel still counts as busy.
/// Deriving this from real receptions keeps a quiet channel fast.
const CCA_PEER_MARGIN_DB: i8 = 10;
const CCA_THRESHOLD_MIN_DBM: i8 = -80;
/// Used when no neighbour has been heard recently: effectively transmit now.
const CCA_THRESHOLD_IDLE_DBM: i8 = -50;
/// A neighbour heard longer ago than this no longer constrains transmission.
const CCA_PEER_TTL: core::time::Duration = core::time::Duration::from_secs(5);

/// How long a silent receiver may go before being re-armed. Trades a rare lost
/// frame for recovery from a stalled receiver.
const RX_REARM_INTERVAL: core::time::Duration = core::time::Duration::from_millis(20);

impl Kaonic1SRadio {
    pub fn new(
        radio: Rf215<SharedBus<Kaonic1SBus>>,
        event: Kaonic1SRadioEvent,
        fem: Kaonic1SRadioFem,
        fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
    ) -> Self {
        let index = fem.index;
        Self {
            radio,
            event: Arc::new(Mutex::new(event)),
            fem,
            bb_frame: BasebandFrame::new(),
            config: RadioConfigBuilder::new().build(),
            modulation: Modulation::Ofdm(OfdmModulation::default()),
            accelerator: Accelerator::Native,
            fpga,
            noise_dbm: -95,
            staged: false,
            index,
            last_rx_arm: Instant::now(),
            last_peer_heard: Instant::now() - CCA_PEER_TTL,
            peer_dbm: 0,
            backoff_window: CCA_BACKOFF_WINDOW_MIN,
            stats: Kaonic1SRadioStats::default(),
            random_state: 0,
        }
    }

    pub fn radio(&mut self) -> &mut Kaonic1SRf215 {
        &mut self.radio
    }

    pub fn event(&self) -> Arc<Mutex<Kaonic1SRadioEvent>> {
        self.event.clone()
    }
}

impl Kaonic1SRadio {

    /// xorshift64*: enough for backoff jitter, and seeded per process so two
    /// devices that defer at the same moment do not pick the same slot.
    /// Jitter for CCA backoff. Not cryptographic — an xorshift seeded from the
    /// clock on first use, which is all that is needed to stop deferred senders
    /// retrying in lock-step.
    ///
    /// Seeding lazily also spreads the two radios apart: constructed together
    /// they would take near-identical clock values, but they first defer at
    /// different moments.
    fn soft_random(&mut self) -> u32 {
        if self.random_state == 0 {
            // `| 1` keeps a seeded state non-zero, so zero stays unambiguously
            // "unseeded" — and an xorshift that reaches zero never leaves it.
            self.random_state = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1;
        }

        let mut x = self.random_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.random_state = x;

        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u32
    }

    /// Counters for this radio.
    pub fn stats(&self) -> Kaonic1SRadioStats {
        self.stats
    }

    /// Current noise-floor estimate in dBm.
    pub fn noise_floor_dbm(&self) -> i8 {
        self.noise_dbm
    }
}

/// Bytes to read out of the RX buffer before the frame ends, overlapping the
/// SPI transfer with the tail of the reception. Measured as neutral on this
/// board (the read-out is not what limits the receiver), so it is off by
/// default and `KAONIC_RX_STREAM_LEVEL` enables it for further testing.
/// Frame-buffer level that triggers early read-out; 0 disables streaming.
pub const RX_STREAM_LEVEL: u16 = 0;

/// The level in effect: `KAONIC_RX_STREAM_LEVEL` overrides the default, read
/// once at start-up. The RF215 has a single receive buffer, so everything not
/// read out by the time the next frame starts arriving is overwritten;
/// reading most of a frame while it is still on the air leaves only its tail
/// to fetch after the end-of-frame interrupt, and that window is what a busy
/// node keeps missing.
pub fn rx_stream_level() -> u16 {
    static LEVEL: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    *LEVEL.get_or_init(|| crate::platform::env::var_or("KAONIC_RX_STREAM_LEVEL", RX_STREAM_LEVEL))
}

/// How long a frame already on the air may take to finish, from the
/// modulation. Capped so a bad rate cannot park the worker.
fn frame_receive_budget(modulation: &Modulation) -> core::time::Duration {
    const MIN: core::time::Duration = core::time::Duration::from_millis(2);
    const MAX: core::time::Duration = core::time::Duration::from_secs(4);
    core::time::Duration::from_micros(modulation.airtime_micros(FRAME_SIZE)).clamp(MIN, MAX)
}

impl Radio for Kaonic1SRadio {
    type TxFrame = Kaonic1SFrame;
    type RxFrame = Kaonic1SFrame;

    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError> {
        log::debug!("set modulation ({}) = {}", self.radio.name(), modulation);

        self.radio.configure(modulation)?;

        self.modulation = *modulation;

        // Reconfiguring drops the receiver out of RX; a node that only
        // listens never transmits, so nothing else would re-arm it.
        let _ = self.radio.start_receive_current();

        Ok(())
    }

    fn get_modulation(&self) -> Modulation {
        self.modulation
    }

    fn set_config(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        self.fem.adjust(config)?;

        log::debug!("set radio config ({}) = {}", self.radio.name(), config);

        self.radio.set_frequency(config)?;

        self.config = *config;

        let _ = self.radio.start_receive_current();

        Ok(())
    }

    fn get_config(&self) -> RadioConfig {
        self.config
    }

    fn update_event(&mut self) -> Result<(), KaonicError> {
        self.radio
            .update_irqs()
            .map_err(|_| KaonicError::HardwareError)?;

        Ok(())
    }

    fn stage(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        self.radio
            .bb_load_tx(&BasebandFrame::new_from_slice(frame.as_slice()))?;
        self.staged = true;
        Ok(())
    }

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        let mut result = Ok(());
        for i in 0..TX_ATTEMPTS {
            let start = Instant::now();

            // The buffer is written once: by `stage`, or by the first
            // attempt. A busy channel leaves it as it is for the retry.
            // The air is claimed for the whole attempt — assessment,
            // frame, and the wait for its end — and only then.
            let attempt = {
                let _air = air::on_air(self.index);
                self.radio
                    .bb_transmit(&BasebandFrame::new_from_slice(frame.as_slice()), self.staged)
            };

            // The receiver holds a frame, finished or still arriving, and
            // only the worker can hand it on: give the transmit back untried.
            // Nothing went on the air, so neither the backoff window nor the
            // counters move, and the receiver is left as it is for the
            // read-out — it is still in RX, so there is nothing to re-arm.
            // The refusal comes before the buffer write, so a staged frame
            // is still staged and an unstaged one still is not.
            if let Err(radio_rf215::error::RadioError::RxPending) = attempt {
                return Err(KaonicError::RxPending);
            }
            self.staged = true;

            // A busy channel is not a failure. Randomised backoff stops every
            // deferred sender colliding again in lock-step.
            if let Err(radio_rf215::error::RadioError::ChannelBusy) = attempt {
                self.stats.cca_busy = self.stats.cca_busy.saturating_add(1);

                if self.stats.cca_busy % 100 == 0 {
                    log::info!(
                        "cca [{}] deferred {} frames (threshold {} dBm, noise {} dBm)",
                        self.radio.name(),
                        self.stats.cca_busy,
                        self.radio.cca_threshold(),
                        self.noise_dbm,
                    );
                }
                self.backoff_window =
                    (self.backoff_window.saturating_mul(2)).min(CCA_BACKOFF_WINDOW_MAX);

                let slots = self.soft_random() % self.backoff_window.max(1);
                std::thread::sleep(CCA_BACKOFF_SLOT * slots);

                result = Err(KaonicError::TryAgain);
                continue;
            }

            result = attempt
                .as_ref()
                .map(|_| ())
                .map_err(|_| KaonicError::HardwareError);

            if let Err(ref err) = attempt {
                log::error!(
                    "tx [{}] attempt {} failed: {:?} (frame {} bytes)",
                    self.radio.name(),
                    i,
                    err,
                    frame.as_slice().len()
                );
                std::thread::sleep(core::time::Duration::from_millis(4));
            } else {
                self.backoff_window = CCA_BACKOFF_WINDOW_MIN;

                self.stats.tx_time_us = self.stats.tx_time_us.saturating_add(start.elapsed().as_micros() as u64);
                self.stats.tx_count = self.stats.tx_count.saturating_add(1);

                if self.stats.tx_count % 100 == 0 {
                    log::info!(
                        "tx [{}] {} frames, {} us/frame in driver",
                        self.radio.name(),
                        self.stats.tx_count,
                        self.stats.tx_micros_per_frame(),
                    );
                }
                log::debug!(
                    "tx [{}] -) |o| {:>4} bytes {:>4}us",
                    self.radio.name(),
                    frame.len(),
                    start.elapsed().as_micros(),
                );

                break;
            }
        }

        // This blocks until the frame has left the antenna, because the
        // transceiver cannot enter RX while still transmitting. Deferring it
        // to the receive path was measured: the transmitter then runs ~40%
        // faster and simply overruns the receiver, so end-to-end throughput
        // drops. It becomes worth revisiting if the receive path gets cheaper.
        let _ = self.radio.start_receive();

        // Sent, or given up on: the next transmit is of another frame.
        self.staged = false;

        result
    }

    fn receive_pending(&mut self) -> Result<bool, KaonicError> {
        Ok(self.radio.rx_pending()?)
    }

    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        let start = Instant::now();

        let result = self
            .radio
            .bb_receive_streaming(
                &mut self.bb_frame,
                timeout,
                frame_receive_budget(&self.modulation),
                rx_stream_level(),
            );

        match result {
            Ok(_) => {
                let edv = self.radio.read_edv().unwrap_or(127);

                let _ = self.radio.start_receive_current();
                self.last_rx_arm = Instant::now();

                // Track the neighbours we can actually hear and assess the
                // channel relative to them.
                if edv > -128 && edv < 0 {
                    self.peer_dbm = if self.peer_dbm == 0 {
                        edv
                    } else {
                        self.peer_dbm.saturating_add((edv - self.peer_dbm) / 4)
                    };

                    self.last_peer_heard = Instant::now();

                    let threshold = self
                        .peer_dbm
                        .saturating_sub(CCA_PEER_MARGIN_DB)
                        .clamp(CCA_THRESHOLD_MIN_DBM, CCA_THRESHOLD_IDLE_DBM);

                    self.radio.set_cca_threshold(threshold);
                }

                log::debug!(
                    "rx [{}] (- |o| {:>4} bytes {:>3}us",
                    self.radio.name(),
                    self.bb_frame.len(),
                    start.elapsed().as_micros(),
                );

                frame.copy_from_slice(self.bb_frame.as_slice());

                Ok(ReceiveResult {
                    rssi: edv,
                    len: self.bb_frame.len(),
                })
            }
            Err(err) => match err {
                radio_rf215::error::RadioError::Timeout => {

                    let interval = RX_REARM_INTERVAL;

                    if !interval.is_zero() && self.last_rx_arm.elapsed() >= interval {
                        self.last_rx_arm = Instant::now();

                        match self.radio.ensure_receive_current() {
                            Ok(true) => {
                                self.stats.rx_rearms = self.stats.rx_rearms.saturating_add(1);

                                if self.stats.rx_rearms % 50 == 0 {
                                    log::info!(
                                        "rx [{}] re-armed {} times (receiver had left RX)",
                                        self.radio.name(),
                                        self.stats.rx_rearms,
                                    );
                                }
                            }
                            Ok(false) => {}
                            Err(_) => {}
                        }
                    }

                    // A neighbour that has gone quiet must not keep the CCA
                    // threshold sensitive forever, or the radio defers to a
                    // station that is no longer there.
                    if self.last_peer_heard.elapsed() >= CCA_PEER_TTL
                        && self.radio.cca_threshold() != CCA_THRESHOLD_IDLE_DBM
                    {
                        self.radio.set_cca_threshold(CCA_THRESHOLD_IDLE_DBM);
                    }

                    return Err(KaonicError::Timeout);
                }
                _ => {
                    log::error!("receive error {}", self.radio.name());

                    let _ = self.radio.start_receive();
                    self.last_rx_arm = Instant::now();

                    return Err(err.into());
                }
            },
        }
    }

    fn scan(&mut self, _timeout: core::time::Duration) -> Result<ScanResult, KaonicError> {
        let rssi = self.radio.read_rssi()?;

        Ok(ScanResult { rssi, snr: 0 })
    }

    fn set_accelerator(&mut self, accelerator: &Accelerator) -> Result<(), KaonicError> {
        match accelerator {
            Accelerator::Hardware => {
                let fpga = self.fpga.clone().ok_or(KaonicError::HardwareError)?;

                // Route I/Q to the FPGA: external loopback + I/Q radio mode, then power it on.
                // self.radio.set_iq_loopback(true)?;
                self.radio.set_mode(ChipMode::Radio)?;
                fpga.lock().unwrap().enable()?;
            }
            Accelerator::Native => {
                // Back to on-chip baseband; power the FPGA down.
                self.radio.set_mode(ChipMode::BasebandRadio)?;
                self.radio.set_iq_loopback(false)?;
                if let Some(fpga) = &self.fpga {
                    fpga.lock().unwrap().disable()?;
                }
            }
        }

        self.accelerator = *accelerator;

        // Switching chip mode resets the baseband core. The radio still reads
        // back as being in RX afterwards, so `ensure_receive` sees nothing
        // wrong and the re-arm watchdog never fires — but the baseband is
        // disabled and no frame is ever decoded. Re-applying the modulation
        // brings the baseband back up, and then the receiver is armed.
        //
        // A radio that carries traffic hides this, because every transmit
        // re-arms it; an idle one simply stops receiving for good, which is
        // what happens to a second radio nothing is sending on.
        self.radio.configure(&self.modulation)?;
        let _ = self.radio.start_receive_current();

        Ok(())
    }

    fn get_accelerator(&self) -> Accelerator {
        self.accelerator
    }

    fn set_antenna(&mut self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        log::debug!(
            "set antenna ({}) {:?} = {:?}",
            self.radio.name(),
            band,
            antenna
        );

        self.fem.set_antenna(band, antenna)
    }

    fn get_antenna(&self, band: RadioBand) -> Antenna {
        self.fem.antenna(band)
    }
}

pub const KAONIC1S_RADIO_COUNT: usize = 2;
pub struct Kaonic1SMachine {
    radios: [Option<Kaonic1SRadio>; KAONIC1S_RADIO_COUNT],
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
}

impl Kaonic1SMachine {
    pub fn new() -> Result<Self, KaonicError> {
        let fpga = Kaonic1SFpga::new()
            .map(|fpga| Arc::new(Mutex::new(fpga)))
            .ok();

        if fpga.is_none() {
            log::warn!("fpga module wasn't found");
        } else {
            log::info!("fpga module found");
        }

        let radios = create_radios(fpga.clone()).map_err(|_| KaonicError::HardwareError)?;

        Ok(Self { radios, fpga })
    }

    pub fn fpga(&self) -> Option<Arc<Mutex<Kaonic1SFpga>>> {
        self.fpga.clone()
    }

    pub fn take_radio(&mut self, index: usize) -> Option<Kaonic1SRadio> {
        if index < self.radios.len() {
            self.radios[index].take()
        } else {
            None
        }
    }

    pub fn for_each_radio<T, F>(
        &mut self,
        mut f: F,
    ) -> Result<[T; KAONIC1S_RADIO_COUNT], KaonicError>
    where
        F: FnMut(usize, &mut Option<Kaonic1SRadio>) -> Result<T, KaonicError>,
        T: Clone,
    {
        let mut results: [Result<T, KaonicError>; KAONIC1S_RADIO_COUNT] = [
            Err(KaonicError::HardwareError),
            Err(KaonicError::HardwareError),
        ];

        for (index, radio) in self.radios.iter_mut().enumerate() {
            results[index] = f(index, radio);
        }

        for r in results.iter() {
            if r.is_err() {
                return Err(KaonicError::IncorrectSettings);
            }
        }

        Ok(results.map(|r| r.unwrap()))
    }
}

//! The radio driven from kernel space: the `kaonic_rf215` driver owns the
//! chip, and this module is its client.
//!
//! The driver presents each of a chip's two transceivers as a network
//! interface carrying whole PHY frames (`rf09_a`, `rf24_a`, `rf09_b`,
//! `rf24_b`), with its settings under `/sys/class/net/<iface>/rf215/`. Frames
//! go through a packet socket and settings through sysfs, so there is no SPI,
//! interrupt or timing work in this process at all. The front-end lines and
//! the FPGA are not the driver's and are driven here, as on the user-space
//! radio.
//!
//! What it costs per frame, which is what matters on one Cortex-A7 core:
//!
//! - receive: one wake-up of the event thread, one `recv` straight into the
//!   caller's frame, one read of the signal strength from a file kept open;
//! - transmit: one `send`. The driver takes the next frame from the socket in
//!   the same pass that finishes the previous one, so back-to-back frames
//!   leave no gap for this process to fill.
//!
//! Each write of a setting has the driver reconfigure the transceiver, which
//! takes it off the air for a moment, so only the settings that actually
//! change are written.

use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use radio_common::{
    modulation::OfdmModulation, Hertz, Modulation, RadioConfig, RadioConfigBuilder,
};

use crate::{
    error::KaonicError,
    platform::kaonic1s::{
        config::{create_fem, radio_configs, RadioBusConfig},
        fem::Kaonic1SRadioFem,
        fpga::Kaonic1SFpga,
        Kaonic1SFrame,
    },
    radio::{Accelerator, Antenna, Radio, RadioBand, ReceiveResult, ScanResult},
};

mod netdev;
mod sysfs;

use netdev::{EdgeWaiter, PacketSocket};
use sysfs::{AttrReader, Attrs};

/// A receive does not wait: the event only fires once a whole frame is on the
/// socket, so a socket that has nothing has nothing coming.
pub const RECEIVE_TIMEOUT: Duration = Duration::ZERO;

/// How long a transmit waits for room when the socket is full, which is how a
/// saturated sender is paced. Longer than the longest frame takes on the air.
const TRANSMIT_TIMEOUT: Duration = Duration::from_secs(1);

/// The interfaces appear when the driver has probed its chip and udev has
/// named them, which at boot can be after this process starts.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_POLL: Duration = Duration::from_millis(100);

/// The driver refuses to bring a link up while the transceiver is mid
/// reception, a moment that passes.
const LINK_UP_ATTEMPTS: usize = 10;
const LINK_UP_RETRY: Duration = Duration::from_millis(50);

/// What the user-space radio reports when it has no measurement.
const RSSI_UNKNOWN: i8 = 127;
/// What the driver reports when it has none.
const RSSI_INVALID: i32 = -128;

/// The settings this module writes, as indices into [`ATTR_PATHS`].
#[derive(Clone, Copy)]
enum Attr {
    Frequency,
    Channel,
    ChannelSpacing,
    Modulation,
    OfdmOpt,
    OfdmMcs,
    QpskFchip,
    QpskMode,
    TxPower,
    Fcs,
}

const ATTR_COUNT: usize = 10;

const ATTR_PATHS: [&str; ATTR_COUNT] = [
    "frequency",
    "channel",
    "channel_spacing",
    "modulation",
    "ofdm/opt",
    "ofdm/mcs",
    "qpsk/fchip",
    "qpsk/mode",
    "frontend/tx_power",
    "fcs",
];

/// Values of [`Attr::Modulation`], which the driver takes by name.
const MODULATION_OFDM: u32 = 0;
const MODULATION_QPSK: u32 = 1;
const MODULATION_NAMES: [&str; 2] = ["ofdm", "qpsk"];

fn band_index(band: RadioBand) -> usize {
    match band {
        RadioBand::Band09 => 0,
        RadioBand::Band24 => 1,
    }
}

/// Which transceiver serves a frequency. The two bands are far apart and a
/// transceiver cannot be tuned out of its own, so the band is a choice of
/// transceiver, not a setting of one.
fn band_of(freq: Hertz) -> Option<RadioBand> {
    match freq.as_hz() {
        389_500_000..=1_020_000_000 => Some(RadioBand::Band09),
        2_400_000_000..=2_483_500_000 => Some(RadioBand::Band24),
        _ => None,
    }
}

fn settings_error(err: io::Error) -> KaonicError {
    match err.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ERANGE) => KaonicError::IncorrectSettings,
        Some(libc::EBUSY) => KaonicError::InvalidState,
        Some(libc::ETIMEDOUT) => KaonicError::Timeout,
        _ => KaonicError::HardwareError,
    }
}

/// One of a chip's two transceivers.
struct Transceiver {
    iface: String,
    socket: PacketSocket,
    attrs: Attrs,
    rssi: AttrReader,
    /// What the driver was last given for each setting, so that a setting
    /// which has not changed is not written again. `None` is unknown.
    applied: [Option<u32>; ATTR_COUNT],
}

impl Transceiver {
    /// `None` when the interface is not there, which is how a transceiver the
    /// device tree disables looks.
    fn open(iface: String) -> Option<Self> {
        let attrs = Attrs::open(&iface)?;

        netdev::disable_ipv6(&iface);

        let socket = PacketSocket::open(&iface)
            .inspect_err(|err| log::error!("{iface}: can't open socket: {err}"))
            .ok()?;

        let rssi = attrs
            .reader("rssi")
            .inspect_err(|err| log::error!("{iface}: can't open rssi: {err}"))
            .ok()?;

        Some(Self {
            iface,
            socket,
            attrs,
            rssi,
            applied: [None; ATTR_COUNT],
        })
    }

    fn set(&mut self, attr: Attr, value: u32) -> Result<(), KaonicError> {
        let index = attr as usize;

        if self.applied[index] == Some(value) {
            return Ok(());
        }

        let result = match attr {
            Attr::Modulation => self
                .attrs
                .write(ATTR_PATHS[index], MODULATION_NAMES[value as usize]),
            _ => self.attrs.write(ATTR_PATHS[index], &value.to_string()),
        };

        match result {
            Ok(()) => {
                self.applied[index] = Some(value);
                Ok(())
            }
            Err(err) => {
                log::warn!(
                    "{}: {} = {} refused: {}",
                    self.iface,
                    ATTR_PATHS[index],
                    value,
                    err
                );
                // The driver keeps the old value when it refuses a new one,
                // but it is cheaper to write once more than to be wrong.
                self.applied[index] = None;
                Err(settings_error(err))
            }
        }
    }

    fn set_config(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        let freq =
            u32::try_from(config.freq.as_hz()).map_err(|_| KaonicError::IncorrectSettings)?;
        let spacing = u32::try_from(config.channel_spacing.as_hz())
            .map_err(|_| KaonicError::IncorrectSettings)?;
        let channel = u32::from(config.channel);

        let wanted = [
            (Attr::Frequency, freq),
            (Attr::ChannelSpacing, spacing),
            (Attr::Channel, channel),
        ];

        let changes = wanted
            .iter()
            .filter(|(attr, value)| self.applied[*attr as usize] != Some(*value))
            .count();

        // The driver checks every write against the settings it already
        // holds, so moving two of these at once can pass through a
        // combination that is out of band and have the move refused half way.
        // With the channel at zero the frequency stands on its own, and the
        // channel goes back on last.
        if changes > 1 {
            self.set(Attr::Channel, 0)?;
        }

        for (attr, value) in wanted {
            self.set(attr, value)?;
        }

        Ok(())
    }

    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError> {
        match modulation {
            Modulation::Ofdm(ofdm) => {
                self.set(Attr::Modulation, MODULATION_OFDM)?;
                self.set(Attr::OfdmOpt, ofdm.opt as u32)?;
                self.set(Attr::OfdmMcs, ofdm.mcs as u32)?;
                self.set(Attr::TxPower, u32::from(ofdm.tx_power))
            }
            Modulation::Qpsk(qpsk) => {
                self.set(Attr::Modulation, MODULATION_QPSK)?;
                self.set(Attr::QpskFchip, qpsk.fchip as u32)?;
                self.set(Attr::QpskMode, qpsk.mode as u32)?;
                self.set(Attr::TxPower, u32::from(qpsk.tx_power))
            }
            Modulation::Off | Modulation::Fsk => Err(KaonicError::NotSupported),
        }
    }

    fn link_down(&self) {
        if let Err(err) = self.socket.set_link(false) {
            log::warn!("{}: can't take the link down: {}", self.iface, err);
        }

        // Taking the link down is reported on the socket; nothing queued
        // before it is wanted either.
        self.socket.drain();
    }

    fn link_up(&self) -> Result<(), KaonicError> {
        for attempt in 0..LINK_UP_ATTEMPTS {
            match self.socket.set_link(true) {
                Ok(()) => return Ok(()),
                Err(err) if err.raw_os_error() == Some(libc::EBUSY) => {
                    if attempt + 1 < LINK_UP_ATTEMPTS {
                        std::thread::sleep(LINK_UP_RETRY);
                    }
                }
                Err(err) => {
                    log::error!("{}: can't bring the link up: {}", self.iface, err);
                    return Err(KaonicError::HardwareError);
                }
            }
        }

        log::error!("{}: link stayed busy", self.iface);

        Err(KaonicError::Timeout)
    }
}

/// Stands in for the interrupt line: wakes when a frame has arrived.
pub struct Kaonic1SRadioEvent {
    waiter: Arc<EdgeWaiter>,
}

impl Kaonic1SRadioEvent {
    pub fn wait_for_event(&mut self, timeout: Option<core::time::Duration>) -> bool {
        self.waiter.wait(timeout)
    }
}

pub struct Kaonic1SRadio {
    name: &'static str,
    /// The chip's transceivers, sub-GHz first.
    bands: [Option<Transceiver>; 2],
    /// Which of them is on the air. The other is held with its link down.
    active: usize,
    fem: Kaonic1SRadioFem,
    event: Arc<Mutex<Kaonic1SRadioEvent>>,
    config: RadioConfig,
    modulation: Modulation,
    accelerator: Accelerator,
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
}

impl Kaonic1SRadio {
    fn new(
        index: usize,
        board: &RadioBusConfig,
        fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
    ) -> Result<Self, KaonicError> {
        // The interfaces are named after the radio, not after the bus it is
        // on, so they are the same on every board revision.
        let suffix = ['a', 'b'][index];
        let bands = [
            Transceiver::open(format!("rf09_{suffix}")),
            Transceiver::open(format!("rf24_{suffix}")),
        ];

        let active = bands
            .iter()
            .position(Option::is_some)
            .ok_or(KaonicError::HardwareError)?;

        let waiter = Arc::new(EdgeWaiter::new().map_err(|_| KaonicError::HardwareError)?);

        for band in bands.iter().flatten() {
            waiter
                .watch(band.socket.as_raw_fd())
                .map_err(|_| KaonicError::HardwareError)?;
        }

        let mut radio = Self {
            name: board.name,
            bands,
            active,
            fem: create_fem(board)?,
            event: Arc::new(Mutex::new(Kaonic1SRadioEvent { waiter })),
            config: RadioConfigBuilder::new().build(),
            modulation: Modulation::Ofdm(OfdmModulation::default()),
            accelerator: Accelerator::Native,
            fpga,
        };

        // Start from a known state whatever a previous run left behind: one
        // transceiver per chip on the air, the other off.
        for band in radio.bands.iter().flatten() {
            band.link_down();
        }

        // The same starting point as the user-space radio, so two boards
        // hear each other out of the box whichever driver each one runs.
        let config = RadioConfigBuilder::new()
            .freq(Hertz::new(869_535_000))
            .channel((index as u16 + 1) * 5)
            .build();

        let modulation = Modulation::Ofdm(OfdmModulation {
            tx_power: 18,
            ..Default::default()
        });

        match band_of(config.freq) {
            Some(band) if radio.bands[band_index(band)].is_some() => {
                radio.active = band_index(band);
                radio.modulation = modulation;
                radio.set_config(&config)?;
            }
            // A chip with only its 2.4 GHz transceiver in use: left on the
            // driver's own settings for that band.
            _ => {
                radio.modulation = modulation;
                radio.apply_modulation()?;
            }
        }

        radio.trx().link_up()?;

        log::info!("radio {} on {}", radio.name, radio.trx().iface);

        Ok(radio)
    }

    pub fn event(&self) -> Arc<Mutex<Kaonic1SRadioEvent>> {
        self.event.clone()
    }

    /// The transceiver on the air.
    fn trx(&self) -> &Transceiver {
        self.bands[self.active]
            .as_ref()
            .expect("the active transceiver exists")
    }

    fn trx_mut(&mut self) -> &mut Transceiver {
        self.bands[self.active]
            .as_mut()
            .expect("the active transceiver exists")
    }

    /// Writes the modulation in force and the frame format to the active
    /// transceiver. The two transceivers keep their settings apart.
    fn apply_modulation(&mut self) -> Result<(), KaonicError> {
        let modulation = self.modulation;
        let trx = self.trx_mut();

        // The radio carries opaque frames with no check sequence of its own,
        // exactly as the user-space radio does; with it on, the two would not
        // hear each other.
        trx.set(Attr::Fcs, 0)?;
        trx.set_modulation(&modulation)
    }

    /// Moves to the chip's other transceiver. The one leaving goes off the
    /// air first and the one arriving is configured before it comes up, so it
    /// is never on the air with another band's settings.
    fn switch_band(&mut self, index: usize, config: &RadioConfig) -> Result<(), KaonicError> {
        self.trx().link_down();

        let leaving = self.active;
        self.active = index;

        let result = self
            .apply_modulation()
            .and_then(|_| self.trx_mut().set_config(config));

        // A band the settings do not fit is not moved to.
        if result.is_err() {
            self.active = leaving;
        }

        log::info!("radio {} on {}", self.name, self.trx().iface);

        self.trx().link_up()?;

        result
    }

    /// Changes the chip-wide I/Q mode, which the driver only allows with both
    /// links down.
    fn set_iq_mode(&mut self, mode: &str) -> Result<(), KaonicError> {
        self.trx().link_down();

        let result = self
            .trx()
            .attrs
            .write("iq/mode", mode)
            .inspect_err(|err| log::error!("{}: iq/mode = {}: {}", self.trx().iface, mode, err))
            .map_err(settings_error);

        self.trx().link_up()?;

        result
    }
}

impl Radio for Kaonic1SRadio {
    type TxFrame = Kaonic1SFrame;
    type RxFrame = Kaonic1SFrame;

    /// The driver services the chip's interrupts; there is nothing to do here.
    fn update_event(&mut self) -> Result<(), KaonicError> {
        Ok(())
    }

    fn set_config(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        let index = band_of(config.freq)
            .map(band_index)
            .ok_or(KaonicError::IncorrectSettings)?;

        if self.bands[index].is_none() {
            return Err(KaonicError::NotSupported);
        }

        self.fem.adjust(config)?;

        log::debug!("set radio config ({}) = {}", self.name, config);

        if index != self.active {
            self.switch_band(index, config)?;
        } else {
            self.apply_modulation()?;
            self.trx_mut().set_config(config)?;
        }

        self.config = *config;

        Ok(())
    }

    fn get_config(&self) -> RadioConfig {
        self.config
    }

    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError> {
        log::debug!("set modulation ({}) = {}", self.name, modulation);

        self.trx_mut().set_modulation(modulation)?;

        self.modulation = *modulation;

        Ok(())
    }

    fn get_modulation(&self) -> Modulation {
        self.modulation
    }

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        let socket = &self.trx().socket;
        let mut deadline = None;

        loop {
            let err = match socket.send(frame.as_slice()) {
                Ok(()) => return Ok(()),
                Err(err) => err,
            };

            match err.raw_os_error() {
                // The socket holds all it may: wait for a frame to leave.
                Some(libc::EAGAIN) => {
                    let deadline =
                        *deadline.get_or_insert_with(|| Instant::now() + TRANSMIT_TIMEOUT);
                    let left = deadline.saturating_duration_since(Instant::now());

                    if left.is_zero() || !socket.wait(libc::POLLOUT, left) {
                        return Err(KaonicError::TryAgain);
                    }
                }
                Some(libc::EINTR) => {}
                Some(libc::ENOBUFS) => return Err(KaonicError::TryAgain),
                Some(libc::EMSGSIZE) => return Err(KaonicError::PayloadTooBig),
                _ => {
                    log::error!("tx [{}] error: {}", self.name, err);
                    return Err(KaonicError::HardwareError);
                }
            }
        }
    }

    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        let trx = self.bands[self.active]
            .as_ref()
            .expect("the active transceiver exists");

        let mut waited = timeout.is_zero();

        loop {
            // Straight into the caller's frame: no buffer of our own in between.
            let received = trx.socket.recv(frame.clear().alloc_max_buffer());

            let err = match received {
                // Nothing to hand on; the driver does not deliver empty frames.
                Ok(0) => continue,
                Ok(len) if len > frame.capacity() => {
                    frame.clear();
                    return Err(KaonicError::PayloadTooBig);
                }
                Ok(len) => {
                    frame.resize(len);

                    // The interface carries no per-frame metadata; the driver
                    // keeps the strength of the last frame it received.
                    let rssi = match trx.rssi.read_i32() {
                        Some(rssi) if rssi != RSSI_INVALID => rssi as i8,
                        _ => RSSI_UNKNOWN,
                    };

                    return Ok(ReceiveResult { rssi, len });
                }
                Err(err) => err,
            };

            frame.clear();

            match err.raw_os_error() {
                Some(libc::EAGAIN) => {
                    if waited || !trx.socket.wait(libc::POLLIN, timeout) {
                        return Err(KaonicError::Timeout);
                    }

                    waited = true;
                }
                Some(libc::EINTR) => {}
                // The link went down, which this radio does itself around a
                // band or mode change. Reading the report clears it.
                Some(libc::ENETDOWN) => return Err(KaonicError::Timeout),
                _ => {
                    log::error!("rx [{}] error: {}", self.name, err);
                    return Err(KaonicError::HardwareError);
                }
            }
        }
    }

    /// Reading the attribute has the driver run one energy measurement on the
    /// channel.
    fn scan(&mut self, _timeout: core::time::Duration) -> Result<ScanResult, KaonicError> {
        let rssi = self.trx().attrs.read_i32("ed").map_err(settings_error)?;

        Ok(ScanResult {
            rssi: rssi.clamp(i8::MIN as i32, i8::MAX as i32) as i8,
            snr: 0,
        })
    }

    fn set_accelerator(&mut self, accelerator: &Accelerator) -> Result<(), KaonicError> {
        match accelerator {
            Accelerator::Hardware => {
                let fpga = self.fpga.clone().ok_or(KaonicError::HardwareError)?;

                // Route I/Q to the FPGA: the chip's own baseband off, its I/Q
                // interface on, then power the FPGA.
                self.set_iq_mode("rf")?;
                fpga.lock().unwrap().enable()?;
            }
            Accelerator::Native => {
                // Back to the on-chip baseband; power the FPGA down.
                self.set_iq_mode("bbrf")?;
                if let Some(fpga) = &self.fpga {
                    fpga.lock().unwrap().disable()?;
                }
            }
        }

        self.accelerator = *accelerator;

        Ok(())
    }

    fn get_accelerator(&self) -> Accelerator {
        self.accelerator
    }

    fn set_antenna(&mut self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        log::debug!("set antenna ({}) {:?} = {:?}", self.name, band, antenna);

        self.fem.set_antenna(band, antenna)
    }

    fn get_antenna(&self, band: RadioBand) -> Antenna {
        self.fem.antenna(band)
    }
}

fn interfaces_present() -> bool {
    ["rf09_a", "rf24_a", "rf09_b", "rf24_b"]
        .iter()
        .any(|iface| Attrs::open(iface).is_some())
}

pub(super) fn create_radios(
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
) -> Result<[Option<Kaonic1SRadio>; 2], KaonicError> {
    let board = radio_configs();

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while !interfaces_present() && Instant::now() < deadline {
        std::thread::sleep(STARTUP_POLL);
    }

    if !interfaces_present() {
        log::error!("no rf215 interfaces; is the kaonic_rf215 driver loaded?");
    }

    let mut radios: [Option<Kaonic1SRadio>; 2] = [None, None];

    for (index, config) in board.iter().enumerate() {
        match Kaonic1SRadio::new(index, config, fpga.clone()) {
            Ok(radio) => radios[index] = Some(radio),
            // Continue with the other radio even if one fails
            Err(err) => log::error!("failed to create radio {}: {:?}", config.name, err),
        }
    }

    Ok(radios)
}

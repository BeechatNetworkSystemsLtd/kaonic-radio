//! The radio driven from user space: the RF215 register driver runs in this
//! process, over spidev and GPIO lines.

use std::{
    sync::{atomic::AtomicUsize, Arc, Mutex},
    time::Instant,
};

use radio_common::{modulation::OfdmModulation, Modulation, RadioConfig, RadioConfigBuilder};
use radio_rf215::{
    baseband::BasebandFrame,
    bus::{BusInterrupt, SpiBus},
    ChipMode, Rf215,
};

use crate::{
    error::KaonicError,
    platform::{
        kaonic1s::{fem::Kaonic1SRadioFem, fpga::Kaonic1SFpga, Kaonic1SFrame},
        linux::{LinuxClock, LinuxGpioInterrupt, LinuxGpioReset, LinuxSpi, SharedBus},
        linux_rf215::AtomicInterrupt,
    },
    radio::{Accelerator, Antenna, Radio, RadioBand, ReceiveResult, ScanResult},
};

mod machine;

pub(super) use machine::create_radios;

/// How long a receive waits for a frame once the interrupt has fired: the
/// frame is still being clocked in when the first interrupt of it arrives.
pub const RECEIVE_TIMEOUT: core::time::Duration = core::time::Duration::from_millis(2);

pub type Kaonic1SBus = SpiBus<LinuxSpi, AtomicInterrupt, LinuxClock, LinuxGpioReset>;

pub type Kaonic1SRf215 = Rf215<SharedBus<Kaonic1SBus>>;

#[derive(Debug)]
pub struct Kaonic1SRadioEvent {
    counter: Arc<AtomicUsize>,
    irq: LinuxGpioInterrupt,
}

impl Kaonic1SRadioEvent {
    pub fn new(counter: Arc<AtomicUsize>, irq: LinuxGpioInterrupt) -> Self {
        Self { counter, irq }
    }

    pub fn wait_for_event(&mut self, timeout: Option<core::time::Duration>) -> bool {
        if self.irq.wait_on_interrupt(timeout) {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
}

impl Kaonic1SRadio {
    pub fn new(
        radio: Rf215<SharedBus<Kaonic1SBus>>,
        event: Kaonic1SRadioEvent,
        fem: Kaonic1SRadioFem,
        fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
    ) -> Self {
        Self {
            radio,
            event: Arc::new(Mutex::new(event)),
            fem,
            bb_frame: BasebandFrame::new(),
            config: RadioConfigBuilder::new().build(),
            modulation: Modulation::Ofdm(OfdmModulation::default()),
            accelerator: Accelerator::Native,
            fpga,
            noise_dbm: -127,
        }
    }

    pub fn radio(&mut self) -> &mut Kaonic1SRf215 {
        &mut self.radio
    }

    pub fn event(&self) -> Arc<Mutex<Kaonic1SRadioEvent>> {
        self.event.clone()
    }
}

impl Radio for Kaonic1SRadio {
    type TxFrame = Kaonic1SFrame;
    type RxFrame = Kaonic1SFrame;

    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError> {
        log::debug!("set modulation ({}) = {}", self.radio.name(), modulation);

        self.radio.configure(modulation)?;

        self.modulation = *modulation;

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

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        let mut result = Ok(());
        for i in 0..4 {
            let start = Instant::now();

            result = self
                .radio
                .bb_transmit(&BasebandFrame::new_from_slice(frame.as_slice()))
                .map_err(|_| KaonicError::HardwareError);

            if result.is_err() {
                log::error!("tx [{}] {} error", self.radio.name(), i);
                std::thread::sleep(core::time::Duration::from_millis(4));
            } else {
                log::debug!(
                    "tx [{}] -) |o| {:>4} bytes {:>4}us",
                    self.radio.name(),
                    frame.len(),
                    start.elapsed().as_micros(),
                );

                break;
            }
        }

        let _ = self.radio.start_receive();

        result
    }

    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        let start = Instant::now();

        let result = self.radio.bb_receive(&mut self.bb_frame, timeout);

        let edv = self.radio.read_edv().unwrap_or(127);

        let _ = self.radio.start_receive();

        match result {
            Ok(_) => {
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
                    let rssi = self.radio.read_rssi().unwrap_or(127);

                    self.noise_dbm = rssi;

                    // log::trace!("RX ({}): RSSI:{}", self.radio.name(), rssi);

                    return Err(KaonicError::Timeout);
                }
                _ => {
                    log::error!("receive error {}", self.radio.name());

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

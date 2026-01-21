use std::sync::{atomic::AtomicUsize, Arc, Mutex};

use radio_rf215::{
    baseband::BasebandFrame,
    bus::{BusInterrupt, SpiBus},
    radio::RadioFrequencyBuilder,
    Rf215,
};

use crate::{
    error::KaonicError,
    frame::Frame,
    modulation::{Modulation, OfdmModulation},
    platform::{
        kaonic1s::machine::create_radios,
        linux::{
            LinuxClock, LinuxGpioInterrupt, LinuxGpioReset, LinuxOutputPin, LinuxSpi, SharedBus,
        },
        linux_rf215::AtomicInterrupt,
        platform_impl::rf215::map_modulation,
    },
    radio::{BandwidthFilter, Hertz, Radio, RadioConfig, ReceiveResult, ScanResult},
};

mod machine;

pub const FRAME_SIZE: usize = 2048usize;

pub type Kaonic1SBus = SpiBus<LinuxSpi, AtomicInterrupt, LinuxClock, LinuxGpioReset>;

pub struct Kaonic1SRadioFem {
    flt_v1: LinuxOutputPin,
    flt_v2: LinuxOutputPin,
    flt_24: LinuxOutputPin,
    ant_24: Option<LinuxOutputPin>,
}

impl Kaonic1SRadioFem {
    pub fn new(
        flt_v1: LinuxOutputPin,
        flt_v2: LinuxOutputPin,
        flt_24: LinuxOutputPin,
        ant_24: Option<LinuxOutputPin>,
    ) -> Self {
        Self {
            flt_v1,
            flt_v2,
            flt_24,
            ant_24,
        }
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

        Ok(())
    }

    pub fn adjust(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        if let Some(ant_24) = &mut self.ant_24 {
            ant_24.set_high()?;
        }

        self.set_bandwidth_filter(config.bandwidth_filter, config.freq)?;

        // NOTE: Should be set to 0
        let _ = self.flt_24.set_low();

        Ok(())
    }
}

pub type Kaonic1SFrame = Frame<FRAME_SIZE>;
pub type Kaonic1SRf215 = Rf215<SharedBus<Kaonic1SBus>>;

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

    modulation: Modulation,

    noise_dbm: i8,
}

impl Kaonic1SRadio {
    pub fn new(
        radio: Rf215<SharedBus<Kaonic1SBus>>,
        event: Kaonic1SRadioEvent,
        fem: Kaonic1SRadioFem,
    ) -> Self {
        Self {
            radio,
            event: Arc::new(Mutex::new(event)),
            fem,
            bb_frame: BasebandFrame::new(),
            modulation: Modulation::Ofdm(OfdmModulation::default()),
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

        let rf_modulation = map_modulation(modulation)?;

        self.radio.configure(&rf_modulation)?;

        self.modulation = *modulation;

        Ok(())
    }

    fn configure(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        self.fem.adjust(config)?;

        log::trace!("set radio config ({}) = {}", self.radio.name(), config);

        self.radio.set_frequency(
            &RadioFrequencyBuilder::new()
                .freq(config.freq.as_hz() as u32)
                .channel_spacing(config.channel_spacing.as_hz() as u32)
                .channel(config.channel)
                .build(),
        )?;

        Ok(())
    }

    fn update_event(&mut self) -> Result<(), KaonicError> {
        self.radio
            .update_irqs()
            .map_err(|_| KaonicError::HardwareError);

        Ok(())
    }

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        log::trace!(
            "tx [{}] -))) |o| {:>4} bytes",
            self.radio.name(),
            frame.len(),
        );

        let result = self
            .radio
            .bb_transmit(&BasebandFrame::new_from_slice(frame.as_slice()))
            .map_err(|_| KaonicError::HardwareError);

        if result.is_err() {
            log::error!("tx [{}] error", self.radio.name());
        }

        result
    }

    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        let result = self.radio.bb_receive(&mut self.bb_frame, timeout);

        let edv = self.radio.read_edv().unwrap_or(127);

        match result {
            Ok(_) => {
                let _ = self.radio.start_receive();

                log::trace!(
                    "rx [{}] (((- |o| {:>4} bytes {:>3}dBm",
                    self.radio.name(),
                    self.bb_frame.len(),
                    edv,
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
}

pub const KAONIC1S_RADIO_COUNT: usize = 2;
pub struct Kaonic1SMachine {
    radios: [Option<Kaonic1SRadio>; KAONIC1S_RADIO_COUNT],
}

impl Kaonic1SMachine {
    pub fn new() -> Result<Self, KaonicError> {
        let radios = create_radios().map_err(|_| KaonicError::HardwareError)?;

        Ok(Self { radios })
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

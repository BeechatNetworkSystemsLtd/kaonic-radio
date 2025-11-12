use radio_rf215::{baseband::BasebandFrame, bus::SpiBus, radio::RadioFrequencyBuilder, Rf215};

use crate::{
    error::KaonicError,
    frame::Frame,
    modulation::{Modulation, OfdmModulation},
    platform::{
        kaonic1s::machine::create_radios,
        linux::{
            LinuxClock, LinuxGpioInterrupt, LinuxGpioReset, LinuxOutputPin, LinuxSpi, SharedBus,
        },
        platform_impl::rf215::map_modulation,
    },
    radio::{self, Radio, ReceiveResult, ScanResult},
};

mod machine;

pub const FRAME_SIZE: usize = 2048usize;

pub type Kaonic1SBus = SpiBus<LinuxSpi, LinuxGpioInterrupt, LinuxClock, LinuxGpioReset>;

pub struct Kaonic1SRadioFem {
    flt_v1: LinuxOutputPin,
    flt_v2: LinuxOutputPin,
    flt_24: LinuxOutputPin,
}

impl Kaonic1SRadioFem {
    pub fn new(flt_v1: LinuxOutputPin, flt_v2: LinuxOutputPin, flt_24: LinuxOutputPin) -> Self {
        Self {
            flt_v1,
            flt_v2,
            flt_24,
        }
    }

    pub fn adjust(&mut self, freq: radio::Hertz) {
        let freq = freq.as_hz();
        if (902_000_000 <= freq) && (freq <= 928_000_000) {
            let _ = self.flt_v1.set_high();
            let _ = self.flt_v2.set_high();
            return;
        }

        if (862_000_000 <= freq) && (freq <= 876_000_000) {
            let _ = self.flt_v1.set_low();
            let _ = self.flt_v2.set_high();
            return;
        }

        if 2_000_000_000 >= freq {
            let _ = self.flt_v1.set_high();
            let _ = self.flt_v2.set_low();
            return;
        }

        if 2_000_000_000 <= freq {
            let _ = self.flt_24.set_high();
            let _ = self.flt_24.set_low();
            return;
        }
    }
}

pub type Kaonic1SFrame = Frame<FRAME_SIZE>;
pub type Kaonic1SRf215 = Rf215<SharedBus<Kaonic1SBus>>;

pub struct Kaonic1SRadio {
    fem: Kaonic1SRadioFem,
    radio: Kaonic1SRf215,
    bb_frame: BasebandFrame,

    modulation: Modulation,

    snr: i8,
    noise_dbm: i8,
}

impl Kaonic1SRadio {
    pub fn new(radio: Rf215<SharedBus<Kaonic1SBus>>, fem: Kaonic1SRadioFem) -> Self {
        Self {
            radio,
            fem,
            bb_frame: BasebandFrame::new(),
            modulation: Modulation::Ofdm(OfdmModulation::default()),
            snr: 0,
            noise_dbm: -127,
        }
    }

    pub fn radio(&mut self) -> &mut Kaonic1SRf215 {
        &mut self.radio
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

    fn configure(&mut self, config: &crate::radio::RadioConfig) -> Result<(), KaonicError> {
        log::trace!("adjust fem to {} Hz", config.freq);

        self.fem.adjust(config.freq);

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

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError> {
        log::trace!("TX ({}): {}B", self.radio.name(), frame.len());

        self.radio
            .bb_transmit(&BasebandFrame::new_from_slice(frame.as_slice()))
            .map_err(|_| KaonicError::HardwareError)?;

        Ok(())
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
                log::trace!(
                    "RX ({}): RSSI:{}dBm {}dBm {}B",
                    self.radio.name(),
                    edv,
                    self.noise_dbm,
                    frame.len()
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
                _ => return Err(err.into()),
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
        let mut results: [Result<T, KaonicError>; KAONIC1S_RADIO_COUNT] =
            [const { Err(KaonicError::HardwareError) }; KAONIC1S_RADIO_COUNT];

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

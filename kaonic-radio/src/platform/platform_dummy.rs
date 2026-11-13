use std::sync::{Arc, Mutex};

use kaonic_frame::frame::Frame;
use radio_common::{modulation::OfdmModulation, Modulation, RadioConfig, RadioConfigBuilder};

use crate::{
    error::KaonicError,
    filter::{self, BandFilter, BandRanges, FilterState, FreqRange},
    radio::{Accelerator, Antenna, Radio, RadioBand, ReceiveResult, ScanResult},
};

/// The dummy radio's filters: like a Kaonic 1S, for working on the API and UI.
const DUMMY_FILTERS: [BandFilter; 3] = [
    BandFilter { label: "Wide 730–960 MHz", min_khz: 730_000, max_khz: 960_000, wide: true },
    BandFilter { label: "868 MHz", min_khz: 862_000, max_khz: 876_000, wide: false },
    BandFilter { label: "915 MHz", min_khz: 902_000, max_khz: 928_000, wide: false },
];
const DUMMY_SUB_GHZ: FreqRange = FreqRange { min_khz: 863_000, max_khz: 928_000 };
const DUMMY_24: FreqRange = FreqRange { min_khz: 2_400_000, max_khz: 2_483_500 };

pub type DummyFrame = Frame<2048>;

pub struct DummyRadioEvent;

impl DummyRadioEvent {
    pub fn wait_for_event(&mut self, timeout: Option<core::time::Duration>) -> bool {
        // No hardware events on the host platform; sleep briefly so the event
        // thread stays responsive to shutdown without busy-looping.
        let sleep = timeout.unwrap_or(core::time::Duration::from_millis(100));
        std::thread::sleep(sleep);
        false
    }
}

pub struct DummyRadio {
    event: Arc<Mutex<DummyRadioEvent>>,
    antennas: [Antenna; 2],
    /// The r30's filter table with no lines behind it, so clients can be
    /// tried against a host build.
    filter: Option<u8>,
}

impl DummyRadio {
    pub fn new() -> Self {
        Self {
            event: Arc::new(Mutex::new(DummyRadioEvent)),
            antennas: [Antenna::default(); 2],
            filter: None,
        }
    }

    fn band_index(band: RadioBand) -> usize {
        match band {
            RadioBand::Band09 => 0,
            RadioBand::Band24 => 1,
        }
    }

    pub fn event(&self) -> Arc<Mutex<DummyRadioEvent>> {
        self.event.clone()
    }
}

pub struct DummyMachine {
    radio_count: usize,
}

impl DummyMachine {
    pub fn new() -> Result<Self, KaonicError> {
        Ok(DummyMachine { radio_count: 2 })
    }

    pub fn take_radio(&mut self, index: usize) -> Option<DummyRadio> {
        if index < self.radio_count {
            Some(DummyRadio::new())
        } else {
            None
        }
    }
}

impl Radio for DummyRadio {
    type TxFrame = DummyFrame;
    type RxFrame = DummyFrame;

    fn update_event(&mut self) -> Result<(), KaonicError> {
        Ok(())
    }

    fn set_config(&mut self, _config: &RadioConfig) -> Result<(), KaonicError> {
        Ok(())
    }

    fn get_config(&self) -> RadioConfig {
        RadioConfigBuilder::new().build()
    }

    fn set_modulation(&mut self, _modulation: &Modulation) -> Result<(), KaonicError> {
        Ok(())
    }

    fn get_modulation(&self) -> Modulation {
        Modulation::Ofdm(OfdmModulation::default())
    }

    fn transmit(&mut self, _frame: &Self::TxFrame) -> Result<(), KaonicError> {
        Err(KaonicError::HardwareError)
    }

    fn receive<'a>(
        &mut self,
        _frame: &'a mut Self::RxFrame,
        _timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        Err(KaonicError::HardwareError)
    }

    fn scan(&mut self, _timeout: core::time::Duration) -> Result<ScanResult, KaonicError> {
        Err(KaonicError::HardwareError)
    }

    fn set_accelerator(&mut self, accelerator: &Accelerator) -> Result<(), KaonicError> {
        // The host platform has no FPGA accelerator.
        match accelerator {
            Accelerator::Native => Ok(()),
            Accelerator::Hardware => Err(KaonicError::NotSupported),
        }
    }

    fn get_accelerator(&self) -> Accelerator {
        Accelerator::Native
    }

    fn set_antenna(&mut self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        self.antennas[Self::band_index(band)] = antenna;
        Ok(())
    }

    fn get_antenna(&self, band: RadioBand) -> Antenna {
        self.antennas[Self::band_index(band)]
    }

    fn get_filter(&self) -> FilterState {
        let config = self.get_config();
        let centre = config.freq.as_hz()
            + u64::from(config.channel) * config.channel_spacing.as_hz();

        FilterState {
            filters: &DUMMY_FILTERS,
            selected: self.filter,
            active: filter::resolve(&DUMMY_FILTERS, self.filter, centre, false),
        }
    }

    fn band_ranges(&self) -> BandRanges {
        BandRanges {
            sub_ghz: Some(DUMMY_SUB_GHZ),
            band_24: Some(DUMMY_24),
        }
    }

    fn set_filter(&mut self, filter: Option<u8>) -> Result<(), KaonicError> {
        if filter.is_some_and(|index| !filter::exists(&DUMMY_FILTERS, index)) {
            return Err(KaonicError::IncorrectSettings);
        }
        self.filter = filter;
        Ok(())
    }
}

pub fn create_machine() -> Result<DummyMachine, KaonicError> {
    DummyMachine::new()
}

pub type PlatformRadio = DummyRadio;
pub type PlatformRadioEvent = DummyRadioEvent;
pub type PlatformRadioFrame = DummyFrame;

/// How long a receive should wait for a frame after the radio's event fired.
pub const PLATFORM_RECEIVE_TIMEOUT: core::time::Duration = core::time::Duration::from_millis(2);

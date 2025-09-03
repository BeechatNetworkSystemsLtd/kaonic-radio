use crate::baseband::{Baseband, BasebandFrame};
use crate::bus::Bus;
use crate::error::RadioError;
use crate::radio::{Band, Radio, RadioChannel, RadioFrequency, RadioFrequencyConfig, RadioState};
use crate::regs::{self, RegisterAddress};

pub struct Band09;
pub struct Band24;

/// sub-GHz Band
impl Band for Band09 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF09_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC0_BASE_ADDRESS;
    const MIN_FREQUENCY: RadioFrequency = 389_500_000;
    const MAX_FREQUENCY: RadioFrequency = 1_020_000_000;
    const FREQUENCY_OFFSET: RadioFrequency = 0;
    const MAX_CHANNEL: RadioChannel = 255;
}

impl Band for Band24 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF24_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC1_BASE_ADDRESS;
    const MIN_FREQUENCY: RadioFrequency = 2_400_000_000;
    const MAX_FREQUENCY: RadioFrequency = 2_483_500_000;
    const FREQUENCY_OFFSET: RadioFrequency = 1_500_000_000;
    const MAX_CHANNEL: RadioChannel = 511;
}

pub struct Transreceiver<B: Band, I: Bus> {
    radio: Radio<B, I>,
    baseband: Baseband<B, I>,
}

impl<B: Band, I: Bus> Transreceiver<B, I> {
    pub(crate) fn new() -> Self {
        Self {
            radio: Radio::<B, I>::new(),
            baseband: Baseband::<B, I>::new(),
        }
    }

    pub fn set_frequency(
        &mut self,
        bus: &mut I,
        config: &RadioFrequencyConfig,
    ) -> Result<(), RadioError> {
        self.radio.set_frequency(bus, config)
    }

    pub fn baseband_transmit(
        &mut self,
        bus: &mut I,
        frame: &BasebandFrame,
    ) -> Result<(), RadioError> {
        self.radio.set_state(bus, RadioState::TrxPrep)?;

        self.radio
            .wait_on_state(bus, RadioState::TrxPrep, core::time::Duration::from_secs(1))?;

        self.baseband.load_tx(bus, frame)?;

        self.radio
            .send_command(bus, crate::radio::RadioCommand::Tx)?;

        Ok(())
    }

    pub fn reset(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.radio.reset(bus)
    }

    pub fn radio(&mut self) -> &mut Radio<B, I> {
        &mut self.radio
    }
}

use core::cell::RefCell;
use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    regs::{
        self, RG_RFXX_CCF0L, RG_RFXX_CMD, RG_RFXX_CNL, RG_RFXX_CNM, RG_RFXX_CS,
        RG_RFXX_FREQ_RESOLUTION_KHZ, RegisterAddress,
    },
};

pub type RadioFrequency = u32;
pub type RadioChannel = u16;

pub struct RadioFrequencyConfig {
    freq: RadioFrequency,
    channel_spacing: RadioFrequency,
    channel: RadioChannel,
}

pub trait Band {
    const RADIO_ADDRESS: RegisterAddress;
    const MIN_FREQUENCY: RadioFrequency;
    const MAX_FREQUENCY: RadioFrequency;
    const FREQUENCY_OFFSET: RadioFrequency;
    const MAX_CHANNEL: RadioChannel;
}

/// Power amplifier current
pub enum Pacur {
    Reduction22mA = 0x00, // 3dB reduction of max. small signal gain
    Reduction18mA = 0x01, // 2dB reduction of max. small signal gain
    Reduction11mA = 0x02, // 1dB reduction of max. small signal gain
    NoReduction = 0x03,   // max. transmit small signal gain
}

#[derive(Debug, PartialEq, Eq)]
pub enum RadioState {
    PowerOff,
    Sleep,
    TrxOff,
    TrxPrep,
    Tx,
    Rx,
    Transition,
}

/// Represents radio module part of the transceiver
/// B is a sub-GHz or 2.4GHz band
pub struct Radio<B, I>
where
    B: Band,
    I: Bus,
{
    state: RadioState,
    _band: PhantomData<B>,
    _bus: PhantomData<I>,
}

impl<B, I> Radio<B, I>
where
    B: Band,
    I: Bus,
{
    pub fn new() -> Self {
        Self {
            state: RadioState::PowerOff,
            _band: PhantomData::default(),
            _bus: PhantomData::default(),
        }
    }

    /// Requests transition into a 'state'
    pub fn set_state(&mut self, bus: &mut I, state: RadioState) -> Result<(), RadioError> {
        let cmd = match state {
            RadioState::PowerOff => 0x00u8,
            RadioState::Sleep => 0x01u8,
            RadioState::TrxOff => 0x02u8,
            RadioState::TrxPrep => 0x03u8,
            RadioState::Tx => 0x04u8,
            RadioState::Rx => 0x05u8,
            RadioState::Transition => return Err(RadioError::IncorrectState),
        };

        self.state = RadioState::Transition;

        bus.write_reg_u8(Self::abs_reg(RG_RFXX_CMD), cmd)
            .map_err(|e| e.into())
    }

    pub fn wait_on_state(
        &mut self,
        _bus: &mut I,
        expected_state: RadioState,
    ) -> Result<(), RadioError> {
        // Can't wait for a transition state
        if expected_state == RadioState::Transition {
            return Err(RadioError::IncorrectState);
        }

        Ok(())
    }

    pub fn wait_interrupt(&mut self, bus: &mut I, timeout: core::time::Duration) -> bool {
        bus.wait_interrupt(timeout)
    }

    /// Configures Radio for a specific frequency, spacing and channel
    pub fn set_frequency(
        &mut self,
        bus: &mut I,
        config: &RadioFrequencyConfig,
    ) -> Result<(), RadioError> {
        self.assert_state(RadioState::TrxOff)?;

        if config.freq < B::MIN_FREQUENCY
            || config.freq > B::MAX_FREQUENCY
            || config.freq < B::FREQUENCY_OFFSET
        {
            return Err(RadioError::IncorrectConfig);
        }

        if config.channel > B::MAX_CHANNEL {
            return Err(RadioError::IncorrectConfig);
        }

        let cs = config.channel_spacing / RG_RFXX_FREQ_RESOLUTION_KHZ;
        if cs > 0xFF {
            return Err(RadioError::IncorrectConfig);
        }

        let freq = (config.freq - B::FREQUENCY_OFFSET) / RG_RFXX_FREQ_RESOLUTION_KHZ;

        bus.write_reg_u8(Self::abs_reg(RG_RFXX_CS), cs as u8)?;

        bus.write_reg_u16(Self::abs_reg(RG_RFXX_CCF0L), freq as u16)?;

        let channel = config.channel.to_le_bytes();

        bus.write_reg_u8(Self::abs_reg(RG_RFXX_CNL), channel[0])?;

        // Using IEEE-compliant Scheme
        bus.write_reg_u8(Self::abs_reg(RG_RFXX_CNM), 0x00 | channel[1])?;

        Ok(())
    }

    /// Set Power Amplifier settings
    pub fn set_pac(&mut self, bus: &mut I, pacur: Pacur, tx_power: u8) -> Result<(), RadioError> {
        let mut value = (pacur as u8) << 5;

        value = value | core::cmp::min(31, tx_power);

        bus.write_reg_u8(regs::RG_RFXX_PAC, value)
            .map_err(|e| e.into())
    }

    pub fn reset(&mut self, bus: &mut I) -> Result<(), RadioError> {
        bus.hardware_reset().map_err(RadioError::from)?;

        self.set_state(bus, RadioState::TrxOff)?;

        Ok(())
    }

    pub fn assert_state(&self, expected_state: RadioState) -> Result<(), RadioError> {
        if self.state != expected_state {
            Err(RadioError::IncorrectState)
        } else {
            Ok(())
        }
    }

    /// Returns absolute register address for a specified `Band`
    const fn abs_reg(addr: RegisterAddress) -> RegisterAddress {
        B::RADIO_ADDRESS + addr
    }
}

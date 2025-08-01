use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    regs::{
        RG_RFXX_CCF0L, RG_RFXX_CMD, RG_RFXX_CNL, RG_RFXX_CNM, RG_RFXX_CS,
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

pub struct RadioInterrupt {
    mask: u8,
}

impl RadioInterrupt {
    pub const fn new() -> Self {
        Self { mask: 0 }
    }
}

pub enum RadioState {
    PowerOff,
    Sleep,
    TrxOff,
    TrxPrep,
    Tx,
    Rx,
}

/// Represents radio module part of the transceiver
/// B is a sub-GHz or 2.4GHz band
pub struct Radio<B, I>
where
    B: Band,
    I: Bus + Copy,
{
    bus: I,
    state: RadioState,
    _band: PhantomData<B>,
}

impl<B, I> Radio<B, I>
where
    B: Band,
    I: Bus + Copy,
{
    pub fn new(bus: I) -> Self {
        Self {
            bus,
            state: RadioState::PowerOff,
            _band: PhantomData::default(),
        }
    }

    /// Requests transition into a 'state'
    fn set_state(&mut self, state: RadioState) -> Result<(), RadioError> {
        let cmd = match state {
            RadioState::PowerOff => 0x00u8,
            RadioState::Sleep => 0x01u8,
            RadioState::TrxOff => 0x02u8,
            RadioState::TrxPrep => 0x03u8,
            RadioState::Tx => 0x04u8,
            RadioState::Rx => 0x05u8,
        };

        self.bus.write_reg_u8(Self::abs_reg(RG_RFXX_CMD), cmd)?;

        Ok(())
    }

    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        match self.state {
            RadioState::TrxOff => return Err(RadioError::IncorrectState),
            _ => (),
        }

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

        self.bus.write_reg_u8(Self::abs_reg(RG_RFXX_CS), cs as u8)?;

        self.bus
            .write_reg_u16(Self::abs_reg(RG_RFXX_CCF0L), freq as u16)?;

        let channel = config.channel.to_le_bytes();

        self.bus
            .write_reg_u8(Self::abs_reg(RG_RFXX_CNL), channel[0])?;

        // Using IEEE-compliant Scheme
        self.bus
            .write_reg_u8(Self::abs_reg(RG_RFXX_CNM), 0x00 | channel[1])?;

        Ok(())
    }

    pub fn reset(&mut self) {
        self.bus.hardware_reset();
    }

    const fn abs_reg(addr: RegisterAddress) -> RegisterAddress {
        B::RADIO_ADDRESS + addr
    }
}

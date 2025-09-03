use core::marker::PhantomData;

use crate::{
    bus::{self, Bus},
    error::RadioError,
    regs::{self, RadioInterruptMask, RegisterAddress},
};

/// Frequency in Hz
pub type RadioFrequency = u32;
pub type RadioChannel = u16;

pub struct RadioFrequencyConfig {
    freq: RadioFrequency,
    channel_spacing: RadioFrequency,
    channel: RadioChannel,
    pll_lbw: PllLoopBandwidth,
}

pub trait Band {
    const RADIO_ADDRESS: RegisterAddress;
    const BASEBAND_ADDRESS: RegisterAddress;
    const MIN_FREQUENCY: RadioFrequency;
    const MAX_FREQUENCY: RadioFrequency;
    const FREQUENCY_OFFSET: RadioFrequency;
    const MAX_CHANNEL: RadioChannel;
}

/// Power amplifier current
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum Pacur {
    Reduction22mA = 0x00 << 5, // 3dB reduction of max. small signal gain
    Reduction18mA = 0x01 << 5, // 2dB reduction of max. small signal gain
    Reduction11mA = 0x02 << 5, // 1dB reduction of max. small signal gain
    NoReduction = 0x03 << 5,   // max. transmit small signal gain
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PllLoopBandwidth {
    Default = 0x00 << 4,
    Smaller = 0x01 << 4, // 15% smaller PLL loopbandwidth
    Larger = 0x02 << 4,  // 15% larger PLL loopbandwidth
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RadioState {
    PowerOff = 0x00,
    Sleep = 0x01,
    TrxOff = 0x02,
    TrxPrep = 0x03,
    Tx = 0x04,
    Rx = 0x05,
    Transition = 0x06,
    Reset = 0x07,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RadioCommand {
    Nop = 0x0,
    Sleep = 0x1,
    TrxOff = 0x2,
    TrxPrep = 0x3,
    Tx = 0x4,
    Rx = 0x5,
    Reset = 0x7,
}

/// Represents radio module part of the transceiver
/// B is a sub-GHz or 2.4GHz band
pub struct Radio<B, I>
where
    B: Band,
    I: Bus,
{
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
            _band: PhantomData::default(),
            _bus: PhantomData::default(),
        }
    }

    pub fn send_command(&mut self, bus: &mut I, command: RadioCommand) -> Result<(), RadioError> {
        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_CMD), command as u8)
            .map_err(|e| e.into())
    }

    /// Requests transition into a 'state'
    pub fn set_state(&mut self, bus: &mut I, state: RadioState) -> Result<(), RadioError> {
        let command = match state {
            RadioState::PowerOff => RadioCommand::Nop,
            RadioState::Sleep => RadioCommand::Sleep,
            RadioState::TrxOff => RadioCommand::TrxOff,
            RadioState::TrxPrep => RadioCommand::TrxPrep,
            RadioState::Tx => RadioCommand::Tx,
            RadioState::Rx => RadioCommand::Rx,
            RadioState::Reset => RadioCommand::Reset,
            RadioState::Transition => return Err(RadioError::IncorrectState),
        };

        self.state = RadioState::Transition;

        self.send_command(bus, command)
    }

    pub fn set_irq_mask(
        &mut self,
        bus: &mut I,
        irq_mask: RadioInterruptMask,
    ) -> Result<(), RadioError> {
        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_IRQM), irq_mask.get())?;
        Ok(())
    }

    pub fn wait_on_state(
        &mut self,
        bus: &mut I,
        expected_state: RadioState,
        timeout: core::time::Duration,
    ) -> Result<(), RadioError> {
        // Can't wait for a transition state
        if expected_state == RadioState::Transition {
            return Err(RadioError::IncorrectState);
        }

        let deadline = (bus.current_time() as u128) + timeout.as_millis();

        loop {
            let state = self.read_state(bus)?;

            if state == expected_state {
                break;
            }

            if (bus.current_time() as u128) > deadline {
                return Err(RadioError::CommunicationFailure);
            }

            self.set_state(bus, expected_state)?;

            bus.delay(core::time::Duration::from_micros(100));
        }

        Ok(())
    }

    pub fn read_state(&mut self, bus: &mut I) -> Result<RadioState, RadioError> {
        let state_value = bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_STATE))?;

        let state = match state_value {
            0x00 => RadioState::PowerOff,
            0x01 => RadioState::Sleep,
            0x02 => RadioState::TrxOff,
            0x03 => RadioState::TrxPrep,
            0x04 => RadioState::Tx,
            0x05 => RadioState::Rx,
            0x06 => RadioState::Transition,
            0x07 => RadioState::Reset,
            _ => return Err(RadioError::IncorrectState),
        };

        Ok(state)
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

        let cs = config.channel_spacing / regs::RG_RFXX_FREQ_RESOLUTION_HZ;
        if cs > 0xFF {
            return Err(RadioError::IncorrectConfig);
        }

        let freq = (config.freq - B::FREQUENCY_OFFSET) / regs::RG_RFXX_FREQ_RESOLUTION_HZ;

        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_CS), cs as u8)?;

        bus.write_reg_u16(Self::abs_reg(regs::RG_RFXX_CCF0L), freq as u16)?;

        let channel = config.channel.to_le_bytes();

        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_CNL), channel[0])?;

        // Using IEEE-compliant Scheme
        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_CNM), 0x00 | channel[1])?;

        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_PLL), config.pll_lbw as u8)?;

        Ok(())
    }

    /// Set Power Amplifier settings
    pub fn set_pac(&mut self, bus: &mut I, pacur: Pacur, tx_power: u8) -> Result<(), RadioError> {
        let mut value = pacur as u8;
        value = value | core::cmp::min(31, tx_power);

        bus.write_reg_u8(Self::abs_reg(regs::RG_RFXX_PAC), value)
            .map_err(|e| e.into())
    }

    pub fn read_rssi(&self, bus: &mut I) -> Result<i8, RadioError> {
        let value = bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_RSSI))?;
        let rssi = value as i8;

        if rssi == 127 {
            return Err(RadioError::IncorrectState);
        }

        Ok(rssi)
    }

    pub fn read_irq(&mut self, bus: &mut I) -> Result<RadioInterruptMask, RadioError> {
        let irq_status = bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_IRQM))?;

        Ok(RadioInterruptMask::new_from_mask(irq_status))
    }

    pub fn clear_irq(&mut self, bus: &mut I) -> Result<(), RadioError> {
        let _ = self.read_irq(bus)?;

        Ok(())
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

use core::fmt;

use bus::{Bus, BusError};
use error::RadioError;
use transceiver::{Band09, Band24, Transreceiver};

use crate::radio::{Band, RadioFrequencyConfig};

pub mod baseband;
pub mod bus;
pub mod error;
pub mod frame;
pub mod modulation;
pub mod radio;
pub mod regs;
pub mod transceiver;

#[derive(PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PartNumber {
    At86Rf215 = 0x34,
    At86Rf215Iq = 0x35,
    At86Rf215M = 0x36,
}

impl fmt::Display for PartNumber {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PartNumber::At86Rf215 => write!(f, "AT86RF215"),
            PartNumber::At86Rf215Iq => write!(f, "AT86RF215IQ"),
            PartNumber::At86Rf215M => write!(f, "AT86RF215M"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum ChipMode {
    BasebandRadio = 0x00, // RF enabled, baseband (BBC0, BBC1) enabled, I/Q IF disabled
    Radio = 0x01,         // RF enabled, baseband (BBC0, BBC1) disabled, I/Q IF enabled
    BasebasendRadio09 = 0x04, // RF enabled, baseband (BBC0) disabled and (BBC1) enabled, I/Q IF for sub-GHz Transceiver enabled
    BasebasendRadio24 = 0x05, // RF enabled, baseband (BBC1) disabled and (BBC0) enabled, I/Q IF for 2.4GHz Transceiver enabled
}

pub struct Rf215<I: Bus + Clone> {
    name: &'static str,
    part_number: PartNumber,
    version: u8,
    bus: I,
    trx_09: Transreceiver<Band09, I>,
    trx_24: Transreceiver<Band24, I>,
}

impl<I: Bus + Clone> Rf215<I> {
    pub fn probe(mut bus: I, name: &'static str) -> Result<Self, BusError> {
        let part_number = bus.read_reg_u8(regs::RG_RF_PN)?;
        let part_number = match part_number {
            0x34 => PartNumber::At86Rf215,
            0x35 => PartNumber::At86Rf215Iq,
            0x36 => PartNumber::At86Rf215M,
            _ => return Err(BusError::CommunicationFailure),
        };

        let version = bus.read_reg_u8(regs::RG_RF_VN)?;

        Ok(Self {
            name,
            part_number,
            version,
            bus: bus.clone(),
            trx_09: Transreceiver::<Band09, I>::new(bus.clone()),
            trx_24: Transreceiver::<Band24, I>::new(bus.clone()),
        })
    }

    pub fn set_iq_loopback(&mut self, enabled: bool) -> Result<(), RadioError> {

        let ext_loopback :u8= if enabled { 0b1000_0000 } else { 0 };

        self.bus.modify_reg_u8(regs::RG_RF_IQIFC0, 0b1000_0000,  ext_loopback)?;

        Ok(())
    }

    pub fn set_mode(&mut self, chip_mode: ChipMode) -> Result<(), RadioError> {
        let chip_mode = (chip_mode as u8) << 4;

        self.bus.modify_reg_u8(regs::RG_RF_IQIFC1, 0b0111_0000, chip_mode)?;

        Ok(())
    }

    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        if config.freq <= Band09::MAX_FREQUENCY {
            self.trx_09.set_frequency(config)
        } else {
            self.trx_24.set_frequency(config)
        }
    }

    pub fn trx_09(&mut self) -> &mut Transreceiver<Band09, I> {
        &mut self.trx_09
    }

    pub fn trx_24(&mut self) -> &mut Transreceiver<Band24, I> {
        &mut self.trx_24
    }

    pub fn reset(&mut self) -> Result<(), RadioError> {
        self.trx_09.reset()?;
        self.trx_24.reset()?;

        Ok(())
    }

    pub fn part_number(&self) -> PartNumber {
        self.part_number
    }

    pub fn version(&self) -> u8 {
        self.version
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

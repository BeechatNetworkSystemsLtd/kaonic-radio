use bus::{Bus, BusError};
use error::RadioError;
use transceiver::{Band09, Band24, Transreceiver};

use crate::radio::Band;

pub mod baseband;
pub mod bus;
pub mod error;
pub mod frame;
pub mod radio;
pub mod regs;
pub mod transceiver;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PartNumber {
    At86Rf215 = 0x34,
    At86Rf215Iq = 0x35,
    At86Rf215M = 0x36,
}

pub struct Rf215<I: Bus> {
    name: &'static str,
    part_number: PartNumber,
    version: u8,
    trx_09: Transreceiver<Band09, I>,
    trx_24: Transreceiver<Band24, I>,
}

impl<I: Bus> Rf215<I> {
    pub fn probe(bus: &mut I, name: &'static str) -> Result<Self, BusError> {
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
            trx_09: Transreceiver::<Band09, I>::new(),
            trx_24: Transreceiver::<Band24, I>::new(),
        })
    }

    pub fn trx_09(&mut self) -> &mut Transreceiver<Band09, I> {
        &mut self.trx_09
    }

    pub fn trx_24(&mut self) -> &mut Transreceiver<Band24, I> {
        &mut self.trx_24
    }

    pub fn reset(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.trx_09.reset(bus)?;
        self.trx_24.reset(bus)?;

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

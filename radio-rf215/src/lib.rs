use core::fmt;

use bus::{Bus, BusError};
use error::RadioError;
use transceiver::{Band09, Band24, Transreceiver};

use crate::{
    baseband::BasebandFrame,
    config::TransreceiverConfigurator,
    modulation::Modulation,
    radio::{RadioFrequencyBuilder, RadioFrequencyConfig},
    regs::{BasebandInterruptMask, RadioInterruptMask},
};

pub mod baseband;
pub mod bus;
pub mod error;
pub mod frame;
pub mod modulation;
pub mod radio;
pub mod regs;
pub mod transceiver;

mod config;

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
    freq_config: RadioFrequencyConfig,
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

        let mut trx_09 = Transreceiver::<Band09, I>::new(bus.clone());
        let mut trx_24 = Transreceiver::<Band24, I>::new(bus.clone());

        trx_09.reset().map_err(|_| BusError::ControlFailure)?;
        trx_24.reset().map_err(|_| BusError::ControlFailure)?;

        let freq_config = RadioFrequencyBuilder::new().build();
        if let Err(_) = trx_09.set_frequency(&freq_config) {
            return Err(BusError::CommunicationFailure);
        }

        Ok(Self {
            name,
            part_number,
            version,
            bus: bus.clone(),
            trx_09,
            trx_24,
            freq_config,
        })
    }

    pub fn bus(&self) -> I {
        self.bus.clone()
    }

    pub fn set_iq_loopback(&mut self, enabled: bool) -> Result<(), RadioError> {
        let ext_loopback: u8 = if enabled { 0b1000_0000 } else { 0 };

        self.bus
            .modify_reg_u8(regs::RG_RF_IQIFC0, 0b1000_0000, ext_loopback)?;

        Ok(())
    }

    pub fn set_mode(&mut self, chip_mode: ChipMode) -> Result<(), RadioError> {
        let chip_mode = (chip_mode as u8) << 4;

        self.bus
            .modify_reg_u8(regs::RG_RF_IQIFC1, 0b0111_0000, chip_mode)?;

        Ok(())
    }

    pub fn setup_irq(
        &mut self,
        radio_irq: RadioInterruptMask,
        baseband_irq: BasebandInterruptMask,
    ) -> Result<(), RadioError> {
        self.trx_09.setup_irq(radio_irq, baseband_irq)?;
        self.trx_24.setup_irq(radio_irq, baseband_irq)?;
        Ok(())
    }

    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        let result = if self.freq_config != *config {
            if self.trx_09.check_band(config.freq) {
                self.trx_09.set_frequency(config)
            } else {
                self.trx_24.set_frequency(config)
            }
        } else {
            Ok(())
        };

        if let Ok(_) = result {
            self.freq_config = *config;
        }

        result
    }

    pub fn configure(&mut self, modulation: &Modulation) -> Result<&mut Self, RadioError> {
        self.trx_09.configure(
            modulation,
            &self.trx_09.create_modulation_config(modulation),
        )?;

        self.trx_24.configure(
            modulation,
            &self.trx_24.create_modulation_config(modulation),
        )?;

        Ok(self)
    }

    pub fn update_irqs(&mut self) -> Result<&mut Self, RadioError> {
        self.trx_09.update_irqs()?;
        self.trx_24.update_irqs()?;
        Ok(self)
    }

    pub fn start_receive(&mut self) -> Result<&mut Self, RadioError> {

        self.trx_09.start_receive()?;
        self.trx_24.start_receive()?;

        Ok(self)
    }

    pub fn bb_transmit(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        if self.trx_09.check_band(self.freq_config.freq) {
            self.trx_09.bb_transmit_cca(frame)
        } else {
            self.trx_24.bb_transmit_cca(frame)
        }
    }

    pub fn bb_receive(
        &mut self,
        frame: &mut BasebandFrame,
        timeout: core::time::Duration,
    ) -> Result<(), RadioError> {
        if self.trx_09.check_band(self.freq_config.freq) {
            self.trx_09.bb_receive(frame, timeout)
        } else {
            self.trx_24.bb_receive(frame, timeout)
        }
    }

    pub fn read_rssi(&mut self) -> Result<i8, RadioError> {
        if self.trx_09.check_band(self.freq_config.freq) {
            self.trx_09.radio().read_rssi()
        } else {
            self.trx_24.radio().read_rssi()
        }
    }

    pub fn read_edv(&mut self) -> Result<i8, RadioError> {
        if self.trx_09.check_band(self.freq_config.freq) {
            self.trx_09.radio().read_edv()
        } else {
            self.trx_24.radio().read_edv()
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

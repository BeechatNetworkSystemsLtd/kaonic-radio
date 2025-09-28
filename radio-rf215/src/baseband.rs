use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    frame::Frame,
    modulation::{Modulation, OfdmModulation},
    radio::Band,
    regs::{self, BasebandInterrupt, BasebandInterruptMask, RegisterAddress, RG_BBCX_FRAME_SIZE},
};

pub type BasebandFrame = Frame<RG_BBCX_FRAME_SIZE>;

pub struct BasebandControl {
    pub enabled: bool,
    pub continuous_tx: bool,
    pub fcs_filter: bool,
}

pub struct Baseband<B, I>
where
    B: Band,
    I: Bus,
{
    _band: PhantomData<B>,
    bus: I,
}

impl<B, I> Baseband<B, I>
where
    B: Band,
    I: Bus,
{
    pub fn new(bus: I) -> Self {
        Self {
            _band: PhantomData::default(),
            bus,
        }
    }

    pub fn setup_irq(&mut self, irq_mask: BasebandInterruptMask) -> Result<(), RadioError> {
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_IRQM), irq_mask.get())?;
        Ok(())
    }

    pub fn load_rx<'a>(
        &mut self,
        frame: &'a mut BasebandFrame,
    ) -> Result<&'a mut BasebandFrame, RadioError> {
        let len = self.bus.read_reg_u16(Self::abs_reg(regs::RG_BBCX_RXFLL))?;

        if len as usize > regs::RG_BBCX_FRAME_SIZE {
            return Err(RadioError::IncorrectState);
        }

        self.bus.read_regs(
            B::BASEBAND_FRAME_BUFFER_ADDRESS + regs::RG_BBCX_FBRXS,
            frame.as_buffer_mut(len as usize),
        )?;

        Ok(frame)
    }

    pub fn load_tx(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        self.load_tx_data(frame.as_slice())
    }

    pub fn load_tx_data(&mut self, data: &[u8]) -> Result<(), RadioError> {
        if data.len() > regs::RG_BBCX_FRAME_SIZE {
            return Err(RadioError::IncorrectState);
        }

        self.bus
            .write_reg_u16(Self::abs_reg(regs::RG_BBCX_TXFLL), data.len() as u16)?;
        self.bus
            .write_regs(B::BASEBAND_FRAME_BUFFER_ADDRESS + regs::RG_BBCX_FBTXS, data)?;

        Ok(())
    }

    pub fn configure(&mut self, modulation: &Modulation) -> Result<(), RadioError> {
        let phy_type: u8 = match modulation {
            Modulation::Off => 0x00,
            Modulation::Fsk => 0x01,
            Modulation::Ofdm(_) => 0x02,
            Modulation::Qpsk => 0x03,
        };

        // Update baseband phy type
        let mut value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_BBCX_PC))?;

        value = (value & 0b1111_1100) | phy_type;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), value)?;

        match modulation {
            Modulation::Off => Ok(()),
            Modulation::Ofdm(ofdm) => self.configure_ofdm(ofdm),
            _ => Err(RadioError::IncorrectConfig),
        }
    }

    pub fn enable(&mut self) -> Result<(), RadioError> {
        self.set_enabled(true)
    }

    pub fn disable(&mut self) -> Result<(), RadioError> {
        self.set_enabled(false)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), RadioError> {
        let mut value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_BBCX_PC))?;

        const BBEN_BIT: u8 = 0b0000_0100;

        if enabled {
            value = value | BBEN_BIT;
        } else {
            value = value & (!BBEN_BIT);
        }

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), value)?;

        Ok(())
    }

    fn configure_ofdm(&mut self, modulation: &OfdmModulation) -> Result<(), RadioError> {
        let phy_config: u8 = modulation.opt as u8;
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMC), phy_config)?;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMPHRTX), modulation.mcs as u8)?;

        let ofdm_switches: u8 = (modulation.pdt << 5) | 0b10000;
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMSW), ofdm_switches)?;

        Ok(())
    }

    pub fn read_irqs(&mut self) -> Result<BasebandInterruptMask, RadioError> {
        let irq_status = self.bus.read_reg_u8(B::BASEBAND_IRQ_ADDRESS)?;
        Ok(BasebandInterruptMask::new_from_mask(irq_status))
    }

    pub fn clear_irq(&mut self) -> Result<(), RadioError> {
        let _ = self.read_irqs()?;
        Ok(())
    }

    pub fn wait_irq(&mut self, irq: BasebandInterrupt, timeout: core::time::Duration) -> bool {
        self.wait_irqs(BasebandInterruptMask::new().add_irq(irq).build(), timeout)
    }

    pub fn wait_irqs(
        &mut self,
        irq_mask: BasebandInterruptMask,
        timeout: core::time::Duration,
    ) -> bool {
        let deadline = self.bus.deadline(timeout);

        loop {
            if self.bus.deadline_reached(deadline) {
                break;
            }

            if self
                .bus
                .wait_interrupt(core::time::Duration::from_micros(100))
            {
                if let Ok(irqs) = self.read_irqs() {
                    if irqs.has_irqs(irq_mask) {
                        return true;
                    }
                }
            }
        }

        return false;
    }

    const fn abs_reg(reg: RegisterAddress) -> RegisterAddress {
        B::BASEBAND_ADDRESS + reg
    }
}

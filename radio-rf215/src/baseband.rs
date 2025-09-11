use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    frame::Frame,
    modulation::{self, Modulation, OfdmModulation},
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
    _bus: PhantomData<I>,
}

impl<B, I> Baseband<B, I>
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

    pub fn setup_irq(
        &mut self,
        bus: &mut I,
        irq_mask: BasebandInterruptMask,
    ) -> Result<(), RadioError> {
        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_IRQM), irq_mask.get())?;
        Ok(())
    }

    pub fn load_rx<'a>(
        &mut self,
        bus: &mut I,
        frame: &'a mut BasebandFrame,
    ) -> Result<&'a mut BasebandFrame, RadioError> {

        let len = bus.read_reg_u16(Self::abs_reg(regs::RG_BBCX_RXFLL))?;

        if len as usize > regs::RG_BBCX_FRAME_SIZE {
            return Err(RadioError::IncorrectState);
        }

        bus.read_regs(
            Self::abs_reg(regs::RG_BBCX_FBRXS),
            frame.as_buffer_mut(len as usize),
        )?;

        Ok(frame)
    }

    pub fn load_tx(&mut self, bus: &mut I, frame: &BasebandFrame) -> Result<(), RadioError> {
        self.load_tx_data(bus, frame.as_slice())
    }

    pub fn load_tx_data(&mut self, bus: &mut I, data: &[u8]) -> Result<(), RadioError> {
        if data.len() > regs::RG_BBCX_FRAME_SIZE {
            return Err(RadioError::IncorrectState);
        }

        bus.write_reg_u16(Self::abs_reg(regs::RG_BBCX_TXFLL), data.len() as u16)?;
        bus.write_regs(Self::abs_reg(regs::RG_BBCX_FBTXS), data)?;

        Ok(())
    }

    pub fn configure(&mut self, bus: &mut I, modulation: &Modulation) -> Result<(), RadioError> {
        let phy_type: u8 = match modulation {
            Modulation::Off => 0x00,
            Modulation::Fsk => 0x01,
            Modulation::Ofdm(_) => 0x02,
            Modulation::Qpsk => 0x03,
        };

        // Update baseband phy type
        let mut value = bus.read_reg_u8(Self::abs_reg(regs::RG_BBCX_PC))?;

        value = (value & 0b1111_1100) | phy_type;

        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), value)?;

        match modulation {
            Modulation::Off => Ok(()),
            Modulation::Ofdm(ofdm) => self.configure_ofdm(bus, ofdm),
            _ => Err(RadioError::IncorrectConfig),
        }
    }

    pub fn enable(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.set_enabled(bus, true)
    }

    pub fn disable(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.set_enabled(bus, false)
    }

    pub fn set_enabled(&mut self, bus: &mut I, enabled: bool) -> Result<(), RadioError> {
        let mut value = bus.read_reg_u8(Self::abs_reg(regs::RG_BBCX_PC))?;

        const BBEN_BIT: u8 = 0b0000_0100;

        if enabled {
            value = value | BBEN_BIT;
        } else {
            value = value & (!BBEN_BIT);
        }

        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), value)?;

        Ok(())
    }

    fn configure_ofdm(
        &mut self,
        bus: &mut I,
        modulation: &OfdmModulation,
    ) -> Result<(), RadioError> {
        let phy_config: u8 = modulation.opt as u8;
        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMC), phy_config)?;

        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMPHRTX), modulation.mcs as u8)?;

        let ofdm_switches: u8 = (modulation.pdt << 5) | 0b10000;
        bus.write_reg_u8(Self::abs_reg(regs::RG_BBCX_OFDMSW), ofdm_switches)?;

        Ok(())
    }

    pub fn read_irqs(&mut self, bus: &mut I) -> Result<BasebandInterruptMask, RadioError> {
        let irq_status = bus.read_reg_u8(B::BASEBAND_IRQ_ADDRESS)?;
        Ok(BasebandInterruptMask::new_from_mask(irq_status))
    }

    pub fn clear_irq(&mut self, bus: &mut I) -> Result<(), RadioError> {
        let _ = self.read_irqs(bus)?;
        Ok(())
    }

    pub fn wait_irq(
        &mut self,
        bus: &mut I,
        irq: BasebandInterrupt,
        timeout: core::time::Duration,
    ) -> bool {
        self.wait_irqs(
            bus,
            BasebandInterruptMask::new().add_irq(irq).build(),
            timeout,
        )
    }

    pub fn wait_irqs(
        &mut self,
        bus: &mut I,
        irq_mask: BasebandInterruptMask,
        timeout: core::time::Duration,
    ) -> bool {
        let deadline = bus.deadline(timeout);

        loop {
            if bus.deadline_reached(deadline) {
                break;
            }

            if bus.wait_interrupt(core::time::Duration::from_micros(100)) {
                if let Ok(irqs) = self.read_irqs(bus) {
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

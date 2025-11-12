use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    frame::Frame,
    modulation::{Modulation, OfdmModulation, QpskModulation},
    radio::Band,
    regs::{self, BasebandInterrupt, BasebandInterruptMask, RegisterAddress, RG_BBCX_FRAME_SIZE},
};

pub type BasebandFrame = Frame<RG_BBCX_FRAME_SIZE>;

pub struct BasebandControl {
    pub enabled: bool,
    pub continuous_tx: bool,
    pub fcs_filter: bool,
}

pub struct BasebandAutoMode {
    pub auto_ack_tx: bool,  // AMCS.AACKFT
    pub auto_ack_fcs: bool, // AMCS.AACKFA
    pub auto_ack_dr: bool,  // AMCS.AACKDR
    pub auto_ack_src: bool, // AMCS.AACKS
    pub auto_ack_en: bool,  // AMCS.AACK
    pub cca_tx: bool,       // AMCS.CCATX
    pub auto_rx: bool,      // AMCS.TX2RX
}

impl Default for BasebandAutoMode {
    fn default() -> Self {
        Self {
            auto_ack_tx: false,
            auto_ack_fcs: false,
            auto_ack_dr: false,
            auto_ack_src: false,
            auto_ack_en: false,
            cca_tx: false,
            auto_rx: false,
        }
    }
}

pub struct Baseband<B, I>
where
    B: Band,
    I: Bus,
{
    _band: PhantomData<B>,
    bus: I,
    enabled: bool,
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
            enabled: false,
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

    pub fn set_auto_mode(&mut self, mode: BasebandAutoMode) -> Result<(), RadioError> {
        let mut amcs = 0u8;

        if mode.auto_ack_tx {
            amcs = amcs | 0b1000_0000;
        }

        if mode.auto_ack_fcs {
            amcs = amcs | 0b0100_0000;
        }

        if mode.auto_ack_dr {
            amcs = amcs | 0b0010_0000;
        }

        if mode.auto_ack_src {
            amcs = amcs | 0b0001_0000;
        }

        if mode.auto_ack_en {
            amcs = amcs | 0b0000_1000;
        }

        if mode.cca_tx {
            amcs = amcs | 0b0000_0010;
        }

        if mode.auto_rx {
            amcs = amcs | 0b0000_0001;
        }

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_AMCS), amcs)?;

        Ok(())
    }

    pub fn set_auto_edt(&mut self, threshold: i8) -> Result<(), RadioError> {
        let amedt: u8 = threshold as u8;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_AMEDT), amedt)?;

        Ok(())
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
            Modulation::Qpsk(_) => 0x03,
        };

        // Update baseband phy type
        let mut value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_BBCX_PC))?;

        value = (value & 0b1111_1100) | phy_type;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), value)?;

        match modulation {
            Modulation::Off => Ok(()),
            Modulation::Ofdm(ofdm) => self.configure_ofdm(ofdm),
            Modulation::Qpsk(qpsk) => self.configure_qpsk(qpsk),
            _ => Err(RadioError::IncorrectConfig),
        }
    }

    pub fn set_fcs(&mut self, enabled: bool) -> Result<(), RadioError> {
        const TXAFCS_BIT: u8 = 0b0001_0000;

        let value = if enabled { TXAFCS_BIT } else { 0 };

        self.bus
            .modify_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), TXAFCS_BIT, value)?;

        Ok(())
    }

    pub fn enable(&mut self) -> Result<(), RadioError> {
        self.set_enabled(true)
    }

    pub fn disable(&mut self) -> Result<(), RadioError> {
        self.set_enabled(false)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), RadioError> {
        if self.enabled == enabled {
            return Ok(());
        }

        const BBEN_BIT: u8 = 0b0000_0100;

        let value = if enabled { BBEN_BIT } else { 0 };

        self.bus
            .modify_reg_u8(Self::abs_reg(regs::RG_BBCX_PC), BBEN_BIT, value)?;

        self.enabled = enabled;

        Ok(())
    }

    pub fn read_counter(&mut self) -> Result<u32, RadioError> {
        let mut bytes = [0u8; 4];

        self.bus
            .read_regs(Self::abs_reg(regs::RG_BBCX_CNT0), &mut bytes[..])?;

        Ok(u32::from_le_bytes(bytes))
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

    fn configure_qpsk(&mut self, modulation: &QpskModulation) -> Result<(), RadioError> {
        self.bus.modify_reg_u8(
            Self::abs_reg(regs::RG_BBCX_OQPSKC0),
            0b0000_0011,
            modulation.fchip as u8,
        )?;

        self.bus.modify_reg_u8(
            Self::abs_reg(regs::RG_BBCX_OQPSKPHRTX),
            0b0000_1110,
            (modulation.mode as u8) << 1,
        )?;

        Ok(())
    }

    pub fn read_irqs(&mut self) -> Result<BasebandInterruptMask, RadioError> {
        let irq_status = self.bus.read_reg_u8(B::BASEBAND_IRQ_ADDRESS)?;
        Ok(BasebandInterruptMask::new_from_mask(irq_status))
    }

    pub fn clear_irqs(&mut self) -> Result<(), RadioError> {
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

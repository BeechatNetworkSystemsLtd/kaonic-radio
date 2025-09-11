use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    frame::Frame,
    radio::Band,
    regs::{self, RegisterAddress, RG_BBCX_FRAME_SIZE},
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

    pub fn load_rx<'a>(
        &mut self,
        bus: &mut I,
        frame: &'a mut BasebandFrame,
    ) -> Result<&'a mut BasebandFrame, RadioError> {
        let len = bus.read_reg_u16(Self::abs_reg(regs::RG_BBCX_FBTXS))?;

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

    pub fn configure(&mut self, bus: &mut I, enabled: bool) -> Result<(), RadioError> {
        Ok(())
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

    const fn abs_reg(reg: RegisterAddress) -> RegisterAddress {
        B::BASEBAND_ADDRESS + reg
    }
}

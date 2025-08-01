use core::time::Duration;

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::{self, SpiDevice};

use crate::regs::{RG_OP_READ, RG_OP_WRITE, RegisterAddress, RegisterValue};

pub enum BusError {
    CommunicationFailure,
    InvalidAddress,
    Timeout,
}

pub trait BusInterrupt {
    fn wait_on_interrupt(&mut self, timeout: Duration) -> bool;
}

pub trait Bus {
    /// Write single register value
    fn write_reg_u8(&mut self, addr: RegisterAddress, value: u8) -> Result<(), BusError> {
        self.write_regs(addr, &[value])
    }

    fn write_reg_u16(&mut self, addr: RegisterAddress, value: u16) -> Result<(), BusError> {
        self.write_regs(addr, &value.to_le_bytes())
    }

    fn read_reg_u8(&mut self, addr: RegisterAddress) -> Result<u8, BusError> {
        let mut values: [RegisterValue; 1] = [0];
        self.read_regs(addr, &mut values)?;
        Ok(values[0])
    }

    fn read_reg_u16(&mut self, addr: RegisterAddress) -> Result<u16, BusError> {
        let mut values: [RegisterValue; 2] = [0, 0];
        self.read_regs(addr, &mut values)?;
        Ok(u16::from_le_bytes(values))
    }

    fn write_regs(
        &mut self,
        addr: RegisterAddress,
        values: &[RegisterValue],
    ) -> Result<(), BusError>;

    fn read_regs(
        &mut self,
        addr: RegisterAddress,
        values: &mut [RegisterValue],
    ) -> Result<(), BusError>;

    fn wait_interrupt(&mut self, timeout: Duration) -> bool;

    /// Helper method to delay for a specific duration
    fn delay(&mut self, timeout: Duration);

    /// Executes hardware reset of RF215 module
    fn hardware_reset(&mut self) -> Result<(), BusError>;
}

pub struct SpiBus<S, I, D, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    D: DelayNs,
    R: OutputPin,
{
    spi: S,
    interrupt: I,
    delay: D,
    reset: R,
}

impl<S, I, D, R> SpiBus<S, I, D, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    D: DelayNs,
    R: OutputPin,
{
    pub fn new(spi: S, interrupt: I, delay: D, reset: R) -> Self {
        Self {
            spi,
            interrupt,
            delay,
            reset,
        }
    }
}

impl<S, I, D, R> Bus for SpiBus<S, I, D, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    D: DelayNs,
    R: OutputPin,
{
    fn write_regs(
        &mut self,
        addr: RegisterAddress,
        values: &[RegisterValue],
    ) -> Result<(), BusError> {
        let addr = (addr | RG_OP_WRITE).to_be_bytes();

        self.spi
            .transaction(&mut [spi::Operation::Write(&addr), spi::Operation::Write(&values)])
            .map_err(|_| BusError::Timeout)
    }

    fn read_regs(
        &mut self,
        addr: RegisterAddress,
        values: &mut [RegisterValue],
    ) -> Result<(), BusError> {
        let addr = (addr | RG_OP_READ).to_be_bytes();

        self.spi
            .transaction(&mut [spi::Operation::Write(&addr), spi::Operation::Read(values)])
            .map_err(|_| BusError::Timeout)
    }

    fn wait_interrupt(&mut self, timeout: Duration) -> bool {
        self.interrupt.wait_on_interrupt(timeout)
    }

    fn delay(&mut self, timeout: Duration) {
        self.delay.delay_ms(timeout.as_millis() as u32);
    }

    fn hardware_reset(&mut self) -> Result<(), BusError> {
        self.reset
            .set_high()
            .map_err(|_| BusError::InvalidAddress)?;

        self.delay(Duration::from_millis(25));

        self.reset.set_low().map_err(|_| BusError::InvalidAddress)?;

        Ok(())
    }
}

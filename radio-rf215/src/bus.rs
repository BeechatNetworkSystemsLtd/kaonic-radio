use core::time::Duration;

use embedded_hal::spi::{self, SpiDevice};

use crate::regs::{RG_OP_READ, RG_OP_WRITE, RegisterAddress, RegisterValue};



#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    CommunicationFailure,
    ControlFailure,
    InvalidAddress,
    Timeout,
}

pub trait BusInterrupt {
    fn wait_on_interrupt(&mut self, timeout: Duration) -> bool;
}

pub trait BusReset {
    fn hardware_reset(&mut self) -> Result<(), BusError>;
}

pub trait BusClock {
    fn delay(&mut self, duration: Duration);

    fn current_time(&mut self) -> u64;
}

pub trait Bus {
    /// Write single register value
    fn write_reg_u8(&mut self, addr: RegisterAddress, value: u8) -> Result<(), BusError> {
        self.write_regs(addr, &[value])
    }

    /// Write word value into register
    fn write_reg_u16(&mut self, addr: RegisterAddress, value: u16) -> Result<(), BusError> {
        self.write_regs(addr, &value.to_le_bytes())
    }

    /// Read single register value
    fn read_reg_u8(&mut self, addr: RegisterAddress) -> Result<u8, BusError> {
        let mut values: [RegisterValue; 1] = [0];
        self.read_regs(addr, &mut values)?;
        Ok(values[0])
    }

    /// Read word value from register
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

    /// Helper method for waiting on event interrupt with timeout
    fn wait_interrupt(&mut self, timeout: Duration) -> bool;

    /// Helper method to delay for a specific duration
    fn delay(&mut self, timeout: Duration);

    /// Helper method to get current time in milliseconds
    fn current_time(&mut self) -> u64;

    /// Executes hardware reset of RF215 module
    fn hardware_reset(&mut self) -> Result<(), BusError>;
}

pub struct SpiBus<S, I, C, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    C: BusClock,
    R: BusReset,
{
    spi: S,
    interrupt: I,
    clock: C,
    reset: R,
}

impl<S, I, C, R> SpiBus<S, I, C, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    C: BusClock,
    R: BusReset,
{
    pub fn new(spi: S, interrupt: I, clock: C, reset: R) -> Self {
        Self {
            spi,
            interrupt,
            clock,
            reset,
        }
    }
}

impl<S, I, C, R> Bus for SpiBus<S, I, C, R>
where
    S: SpiDevice,
    I: BusInterrupt,
    C: BusClock,
    R: BusReset,
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
        self.clock.delay(timeout);
    }

    fn current_time(&mut self) -> u64 {
        self.clock.current_time()
    }

    fn hardware_reset(&mut self) -> Result<(), BusError> {
        self.reset.hardware_reset()
    }
}

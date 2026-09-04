use libgpiod::line::Value;
use radio_rf215::bus::Bus;
use radio_rf215::bus::BusClock;
use radio_rf215::bus::BusError;
use radio_rf215::bus::BusInterrupt;
use radio_rf215::bus::BusReset;
use radio_rf215::error::RadioError;

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;

use super::linux::LinuxClock;
use super::linux::LinuxGpioReset;
use super::linux::SharedBus;
use crate::error::KaonicError;
use crate::platform::linux::LinuxGpioInterrupt;

impl<T: Bus> Bus for SharedBus<T> {
    #[inline]
    fn write_regs(
        &mut self,
        addr: radio_rf215::regs::RegisterAddress,
        values: &[radio_rf215::regs::RegisterValue],
    ) -> Result<(), BusError> {
        let mut bus = self.bus.lock().unwrap();
        bus.write_regs(addr, values)
    }

    #[inline]
    fn read_regs(
        &mut self,
        addr: radio_rf215::regs::RegisterAddress,
        values: &mut [radio_rf215::regs::RegisterValue],
    ) -> Result<(), BusError> {
        let mut bus = self.bus.lock().unwrap();
        bus.read_regs(addr, values)
    }

    #[inline]
    fn irq_poll(
        &mut self,
        reg: radio_rf215::regs::RegisterAddress,
    ) -> Result<u8, BusError> {
        let mut bus = self.bus.lock().unwrap();
        bus.irq_poll(reg)
    }

    #[inline]
    fn wait_interrupt(&mut self, timeout: Option<std::time::Duration>) -> bool {
        // Never hold the bus mutex while sleeping: it would stall SPI access.
        if let Some(irq) = &self.irq {
            let (count, signaled) = irq.wait(self.irq_count, timeout);
            self.irq_count = count;
            return signaled;
        }

        let mut bus = self.bus.lock().unwrap();
        bus.wait_interrupt(timeout)
    }

    #[inline]
    fn delay(&mut self, timeout: std::time::Duration) {
        let mut bus = self.bus.lock().unwrap();
        bus.delay(timeout)
    }

    #[inline]
    fn current_time(&mut self) -> u64 {
        let mut bus = self.bus.lock().unwrap();
        bus.current_time()
    }

    #[inline]
    fn hardware_reset(&mut self) -> Result<(), BusError> {
        let mut bus = self.bus.lock().unwrap();
        bus.hardware_reset()
    }
}

impl BusInterrupt for LinuxGpioInterrupt {
    fn wait_on_interrupt(&mut self, timeout: Option<core::time::Duration>) -> bool {
        if let Ok(status) = self.request.wait_edge_events(timeout) {
            if status {
                let _ = self.request.read_edge_events(&mut self.buffer);
            }

            return status;
        }

        return false;
    }
}

impl BusReset for LinuxGpioReset {
    fn hardware_reset(&mut self) -> Result<(), BusError> {
        self.request
            .set_value(self.line, Value::Active)
            .map_err(|_| BusError::ControlFailure)?;

        std::thread::sleep(std::time::Duration::from_millis(25));

        self.request
            .set_value(self.line, Value::InActive)
            .map_err(|_| BusError::ControlFailure)?;

        std::thread::sleep(std::time::Duration::from_millis(25));

        Ok(())
    }
}

impl BusClock for LinuxClock {
    fn delay(&mut self, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }

    fn current_time(&mut self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

/// Hand-off between the GPIO edge-wait thread and blocking waiters.
#[derive(Debug, Default)]
pub struct IrqSignal {
    count: Mutex<usize>,
    condvar: Condvar,
}

impl IrqSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn notify(&self) {
        {
            let mut count = self.count.lock().unwrap();
            *count = count.wrapping_add(1);
        }

        // Notify outside the lock so woken waiters don't block on it.
        self.condvar.notify_all();
    }

    /// Returns the observed counter value and whether it advanced past `prev`.
    pub(crate) fn wait(&self, prev: usize, timeout: Option<core::time::Duration>) -> (usize, bool) {
        let count = self.count.lock().unwrap();

        match timeout {
            Some(timeout) => {
                let (count, result) = self
                    .condvar
                    .wait_timeout_while(count, timeout, |c| *c == prev)
                    .unwrap();

                (*count, !result.timed_out())
            }
            None => {
                let count = self.condvar.wait_while(count, |c| *c == prev).unwrap();

                (*count, true)
            }
        }
    }
}

#[derive(Debug)]
pub struct AtomicInterrupt {
    signal: Arc<IrqSignal>,
    prev_count: usize,
}

impl AtomicInterrupt {
    pub fn new(signal: Arc<IrqSignal>) -> Self {
        Self {
            signal,
            prev_count: 0,
        }
    }
}

impl BusInterrupt for AtomicInterrupt {
    fn wait_on_interrupt(&mut self, timeout: Option<core::time::Duration>) -> bool {
        let (count, signaled) = self.signal.wait(self.prev_count, timeout);
        self.prev_count = count;
        signaled
    }
}

impl From<RadioError> for KaonicError {
    fn from(value: RadioError) -> Self {
        match value {
            RadioError::IncorrectConfig => Self::IncorrectSettings,
            RadioError::IncorrectState => Self::HardwareError,
            RadioError::CommunicationFailure => Self::HardwareError,
            RadioError::Timeout => Self::Timeout,
            RadioError::ChannelBusy => Self::TryAgain,
        }
    }
}

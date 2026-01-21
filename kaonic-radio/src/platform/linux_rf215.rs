use libgpiod::line::Value;
use radio_rf215::bus::Bus;
use radio_rf215::bus::BusClock;
use radio_rf215::bus::BusError;
use radio_rf215::bus::BusInterrupt;
use radio_rf215::bus::BusReset;
use radio_rf215::error::RadioError;

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
    fn wait_interrupt(&mut self, timeout: Option<std::time::Duration>) -> bool {
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

            return true;
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

impl From<RadioError> for KaonicError {
    fn from(value: RadioError) -> Self {
        match value {
            RadioError::IncorrectConfig => Self::IncorrectSettings,
            RadioError::IncorrectState => Self::HardwareError,
            RadioError::CommunicationFailure => Self::HardwareError,
            RadioError::Timeout => Self::Timeout,
        }
    }
}

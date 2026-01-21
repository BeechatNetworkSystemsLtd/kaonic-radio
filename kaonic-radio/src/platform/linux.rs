use std::time::Instant;

use libgpiod::line::Bias;
use libgpiod::line::Offset;
use libgpiod::line::Value;

use linux_embedded_hal::SpidevDevice;

use crate::error::KaonicError;

pub struct SharedBus<T> {
    pub(super) bus: std::sync::Arc<std::sync::Mutex<T>>,
}

impl<T> SharedBus<T> {
    /// Create a new `SharedDevice`.
    #[inline]
    pub fn new(bus: std::sync::Arc<std::sync::Mutex<T>>) -> Self {
        Self { bus }
    }
}

impl<T> Clone for SharedBus<T> {
    fn clone(&self) -> Self {
        Self {
            bus: self.bus.clone(),
        }
    }
}

pub struct LinuxGpioConfig {
    pub line_name: &'static str,
}

pub struct LinuxGpioLineConfig {
    pub chip: &'static str,
    pub offset: u32,
}

pub struct LinuxSpiConfig {
    pub path: &'static str,
    pub max_speed: u32,
}

pub struct LinuxGpioInterrupt {
    pub(super) buffer: libgpiod::request::Buffer,
    pub(super) request: libgpiod::request::Request,
}

pub type LinuxSpi = SpidevDevice;

impl LinuxGpioInterrupt {
    pub fn new(line_name: &str, name: &str) -> Result<Self, KaonicError> {
        let gpio = create_gpio_by_name(&format!("{}-rf215-irq", name), line_name, {
            let mut settings = libgpiod::line::Settings::new()?;
            settings.set_bias(Some(Bias::PullDown))?;
            settings.set_edge_detection(Some(libgpiod::line::Edge::Rising))?;
            settings.set_event_clock(libgpiod::line::EventClock::Realtime)?;
            settings
        })?;

        let buffer = libgpiod::request::Buffer::new(1)?;

        Ok(Self {
            request: gpio.1,
            buffer,
        })
    }
}

pub struct LinuxGpioReset {
    pub(super) line: Offset,
    pub(super) request: libgpiod::request::Request,
}

impl LinuxGpioReset {
    pub fn new(line_name: &str, name: &str) -> Result<Self, KaonicError> {
        let gpio = create_gpio_by_name(&format!("{}-rf215-rst", name), line_name, {
            let mut settings = libgpiod::line::Settings::new()?;
            settings.set_direction(libgpiod::line::Direction::Output)?;
            settings.set_output_value(Value::InActive)?;
            settings.set_active_low(true);
            settings
        })?;

        Ok(Self {
            line: gpio.0,
            request: gpio.1,
        })
    }
}

pub struct LinuxOutputPin {
    line: Offset,
    request: libgpiod::request::Request,
}

impl LinuxOutputPin {
    pub fn new(line_name: &str, name: &str) -> Result<Self, KaonicError> {
        let gpio = create_gpio_by_name(name, line_name, {
            let mut settings = libgpiod::line::Settings::new()?;
            settings.set_direction(libgpiod::line::Direction::Output)?;
            settings.set_output_value(Value::InActive)?;
            settings.set_active_low(false);
            settings
        })?;

        Ok(Self {
            line: gpio.0,
            request: gpio.1,
        })
    }

    pub fn new_from_line(chip: &'static str, offset: u32, name: &str) -> Result<Self, KaonicError> {
        let gpio = create_gpio_by_line(name, LinuxGpioLineConfig { chip, offset }, {
            let mut settings = libgpiod::line::Settings::new()?;
            settings.set_direction(libgpiod::line::Direction::Output)?;
            settings.set_output_value(Value::InActive)?;
            settings.set_active_low(false);
            settings
        })?;

        Ok(Self {
            line: gpio.0,
            request: gpio.1,
        })
    }

    pub fn set_high(&mut self) -> Result<(), KaonicError> {
        self.request
            .set_value(self.line, Value::Active)
            .map(|_| {})
            .map_err(|_| KaonicError::HardwareError)
    }

    pub fn set_low(&mut self) -> Result<(), KaonicError> {
        self.request
            .set_value(self.line, Value::InActive)
            .map(|_| {})
            .map_err(|_| KaonicError::HardwareError)
    }
}

pub struct LinuxClock {
    pub(crate) start_time: Instant,
}

impl LinuxClock {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }
}

fn create_gpio_by_line(
    name: &str,
    line: LinuxGpioLineConfig,
    line_settings: libgpiod::line::Settings,
) -> Result<(Offset, libgpiod::request::Request), KaonicError> {
    let chip = libgpiod::chip::Chip::open(&line.chip)?;

    let mut line_config = libgpiod::line::Config::new()?;
    line_config.add_line_settings(&[line.offset], line_settings)?;

    let mut req_config = libgpiod::request::Config::new()?;

    let request = chip.request_lines(Some(req_config.set_consumer(name)?), &line_config)?;

    return Ok((line.offset, request));
}

fn create_gpio_by_name(
    name: &str,
    line_name: &str,
    line_settings: libgpiod::line::Settings,
) -> Result<(Offset, libgpiod::request::Request), KaonicError> {
    for chip in libgpiod::gpiochip_devices(&"/dev")? {
        let offset = chip.line_offset_from_name(line_name);

        if let Ok(offset) = offset {
            let mut line_config = libgpiod::line::Config::new()?;
            line_config.add_line_settings(&[offset], line_settings)?;

            let mut req_config = libgpiod::request::Config::new()?;

            let request = chip.request_lines(Some(req_config.set_consumer(name)?), &line_config)?;

            return Ok((offset, request));
        } else {
        }
    }

    log::error!(
        "gpio line with name '{}' not found (for {})",
        line_name,
        name
    );

    Err(KaonicError::HardwareError)
}

impl From<libgpiod::Error> for KaonicError {
    fn from(_value: libgpiod::Error) -> Self {
        Self::HardwareError
    }
}

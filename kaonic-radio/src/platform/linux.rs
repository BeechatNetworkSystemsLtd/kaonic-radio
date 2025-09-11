use std::time::Duration;
use std::time::Instant;

use libgpiod::line::Offset;
use libgpiod::line::Value;

use libgpiod::Error;
use linux_embedded_hal::SpidevDevice;

use radio_rf215::bus::BusClock;
use radio_rf215::bus::BusError;
use radio_rf215::bus::BusInterrupt;
use radio_rf215::bus::BusReset;
use radio_rf215::bus::SpiBus;

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
    buffer: libgpiod::request::Buffer,
    request: libgpiod::request::Request,
}

pub type LinuxSpi = SpidevDevice;

impl LinuxGpioInterrupt {
    pub fn new(line_name: &str, name: &str) -> Result<Self, libgpiod::Error> {
        let gpio = create_gpio_by_name(&format!("{}-rf215-irq", name), line_name, {
            let mut settings = libgpiod::line::Settings::new()?;
            settings.set_edge_detection(Some(libgpiod::line::Edge::Falling))?;
            settings
        })?;

        let buffer = libgpiod::request::Buffer::new(1)?;

        Ok(Self {
            request: gpio.1,
            buffer,
        })
    }
}

impl BusInterrupt for LinuxGpioInterrupt {
    fn wait_on_interrupt(&mut self, timeout: core::time::Duration) -> bool {
        if let Ok(status) = self.request.wait_edge_events(Some(timeout)) {
            if status {
                let _ = self.request.read_edge_events(&mut self.buffer);
            }

            return true;
        }

        return false;
    }
}

pub struct LinuxGpioReset {
    line: Offset,
    request: libgpiod::request::Request,
}

impl LinuxGpioReset {
    pub fn new(line_name: &str, name: &str) -> Result<Self, libgpiod::Error> {
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
    pub fn new(line_name: &str, name: &str) -> Result<Self, libgpiod::Error> {
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

    pub fn new_from_line(chip: &'static str, offset: u32, name: &str) -> Result<Self, libgpiod::Error> {
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

    pub fn set_high(&mut self) -> Result<(), libgpiod::Error> {
        self.request.set_value(self.line, Value::Active).map(|_| {})
    }

    pub fn set_low(&mut self) -> Result<(), libgpiod::Error> {
        self.request
            .set_value(self.line, Value::InActive)
            .map(|_| {})
    }
}

impl BusReset for LinuxGpioReset {
    fn hardware_reset(&mut self) -> Result<(), BusError> {
        self.request
            .set_value(self.line, Value::Active)
            .map_err(|_| BusError::ControlFailure)?;

        std::thread::sleep(Duration::from_millis(25));

        self.request
            .set_value(self.line, Value::InActive)
            .map_err(|_| BusError::ControlFailure)?;

        Ok(())
    }
}

pub struct LinuxClock {
    start_time: Instant,
}

impl LinuxClock {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }
}

impl BusClock for LinuxClock {
    fn delay(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn current_time(&mut self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

fn create_gpio_by_line(
    name: &str,
    line: LinuxGpioLineConfig,
    line_settings: libgpiod::line::Settings,
) -> Result<(Offset, libgpiod::request::Request), libgpiod::Error> {
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
) -> Result<(Offset, libgpiod::request::Request), libgpiod::Error> {
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

    Err(Error::IoError)
}

use linux::{LinuxGpioConfig, LinuxSpiConfig};
use linux_embedded_hal::spidev::SpidevOptions;
use radio_rf215::{
    bus::{BusError, SpiBus},
    Rf215,
};

use crate::RadioModule;

mod linux;

pub type PlatformBus =
    SpiBus<linux::LinuxSpi, linux::LinuxGpioInterrupt, linux::LinuxClock, linux::LinuxGpioReset>;

struct RadioBusConfig {
    name: &'static str,
    rst_gpio: LinuxGpioConfig,
    irq_gpio: LinuxGpioConfig,
    spi: LinuxSpiConfig,
}

const RADIO_CONFIG_REV_A: [RadioBusConfig; 2] = [
    RadioBusConfig {
        name: "rfa",
        rst_gpio: LinuxGpioConfig { line_name: "PD8" },
        irq_gpio: LinuxGpioConfig { line_name: "PD9" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev6.0",
            max_speed: 5_000_000,
        },
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 5_000_000,
        },
    },
];

const RADIO_CONFIG_REV_B: [RadioBusConfig; 2] = [
    RadioBusConfig {
        name: "rfa",
        rst_gpio: LinuxGpioConfig { line_name: "PD8" },
        irq_gpio: LinuxGpioConfig { line_name: "PD9" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev6.0",
            max_speed: 5_000_000,
        },
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 5_000_000,
        },
    },
];

const RADIO_CONFIG_REV_C: [RadioBusConfig; 2] = RADIO_CONFIG_REV_B;

pub fn create_radios() -> Result<[Option<RadioModule<PlatformBus>>; 2], BusError> {
    // Read machine configuration from /etc/kaonic/kaonic_machine
    let machine_config = match std::fs::read_to_string("/etc/kaonic/kaonic_machine") {
        Ok(content) => content.trim().to_string(),
        Err(e) => {
            log::warn!(
                "Failed to read /etc/kaonic/kaonic_machine: {}, using default config",
                e
            );
            "stm32mp1-kaonic-protoa".to_string() // Default fallback
        }
    };

    log::info!("Machine configuration: {}", machine_config);

    // Select radio configuration based on machine type
    let radio_configs = match machine_config.as_str() {
        "stm32mp1-kaonic-protoa" => &RADIO_CONFIG_REV_A,
        "stm32mp1-kaonic-protob" => &RADIO_CONFIG_REV_B,
        "stm32mp1-kaonic-protoc" => &RADIO_CONFIG_REV_C,
        _ => {
            log::warn!(
                "Unknown machine configuration '{}', using rev_a as default",
                machine_config
            );
            &RADIO_CONFIG_REV_A
        }
    };

    let mut radios: [Option<RadioModule<PlatformBus>>; 2] = [None, None];

    // Create radios based on selected configuration
    for (index, config) in radio_configs.iter().enumerate() {
        match create_radio(config) {
            Ok(radio) => {
                radios[index] = Some(radio);
            }
            Err(_e) => {
                log::error!("Failed to create radio {}", config.name);
                // Continue with other radios even if one fails
            }
        }
    }

    Ok(radios)
}

fn create_radio(config: &RadioBusConfig) -> Result<RadioModule<PlatformBus>, BusError> {
    // Create SPI interface
    let mut spi = linux::LinuxSpi::open(&config.spi.path).map_err(|_| BusError::ControlFailure)?;

    spi.configure(
        &SpidevOptions::new()
            .max_speed_hz(config.spi.max_speed)
            .build(),
    )
    .map_err(|_| BusError::ControlFailure)?;

    // Create GPIO interfaces
    let reset_gpio = linux::LinuxGpioReset::new(&config.rst_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;
    let interrupt_gpio = linux::LinuxGpioInterrupt::new(&config.irq_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    // Create clock (system clock)
    let clock = linux::LinuxClock::new();

    // Create the bus with all interfaces
    let mut bus = SpiBus::new(spi, interrupt_gpio, clock, reset_gpio);

    // Probe and initialize the RF215
    let radio = Rf215::probe(&mut bus, config.name)?;

    Ok(RadioModule::new(bus, radio))
}

//! What differs between board revisions: where each radio is wired.
//!
//! The reset, interrupt and SPI entries are only used by the user-space radio;
//! the kernel-space radio gets the same wiring from the device tree. The
//! front-end lines are used by both.

use crate::{
    error::KaonicError,
    platform::{
        kaonic1s::fem::Kaonic1SRadioFem,
        linux::{LinuxGpioConfig, LinuxGpioLineConfig, LinuxOutputPin, LinuxSpiConfig},
    },
};

const MACHINE_FILE: &str = "/etc/kaonic/kaonic_machine";

#[cfg_attr(feature = "machine-kaonic1s-ks", allow(dead_code))]
pub(super) struct RadioBusConfig {
    pub name: &'static str,
    pub rst_gpio: LinuxGpioConfig,
    pub irq_gpio: LinuxGpioConfig,
    pub spi: LinuxSpiConfig,
    pub flt_v1_gpio: LinuxGpioLineConfig,
    pub flt_v2_gpio: LinuxGpioLineConfig,
    pub flt_24_gpio: Option<LinuxGpioLineConfig>,
    pub ant_24_gpio: Option<LinuxGpioLineConfig>,
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
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 10,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 11,
        },
        flt_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 12,
        }),
        ant_24_gpio: None,
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 5_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 1,
        },
        flt_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 2,
        }),
        ant_24_gpio: None,
    },
];

const RADIO_CONFIG_REV_B: [RadioBusConfig; 2] = [
    RadioBusConfig {
        name: "rfa",
        rst_gpio: LinuxGpioConfig { line_name: "PD8" },
        irq_gpio: LinuxGpioConfig { line_name: "PD9" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev6.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 10,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 11,
        },
        flt_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 12,
        }),
        ant_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 13,
        }),
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 1,
        },
        flt_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 2,
        }),
        ant_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 14,
        }),
    },
];

const RADIO_CONFIG_REV_C: [RadioBusConfig; 2] = RADIO_CONFIG_REV_B;
const RADIO_CONFIG_REV_24: [RadioBusConfig; 2] = RADIO_CONFIG_REV_C;

const RADIO_CONFIG_REV_30: [RadioBusConfig; 2] = [
    RadioBusConfig {
        name: "rfa",
        rst_gpio: LinuxGpioConfig { line_name: "PD8" },
        irq_gpio: LinuxGpioConfig { line_name: "PD9" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev4.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 10,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 11,
        },
        flt_24_gpio: None,
        ant_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 13,
        }),
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 1,
        },
        flt_24_gpio: None,
        ant_24_gpio: Some(LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 14,
        }),
    },
];

// kaonic1x-r09 has four I/O expanders where r30 has two, so the 16-bit one
// (I2C2 @ 0x21) that carries the filter selects and FPGA_EN enumerates as
// gpiochip11 instead of gpiochip9. Its line offsets are unchanged.
const GPIO_EXP0_1X_REV_09: &str = "/dev/gpiochip11";

const RADIO_CONFIG_1X_REV_09: [RadioBusConfig; 2] = [
    RadioBusConfig {
        name: "rfa",
        rst_gpio: LinuxGpioConfig { line_name: "PD8" },
        irq_gpio: LinuxGpioConfig { line_name: "PD9" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev4.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: GPIO_EXP0_1X_REV_09,
            offset: 10,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: GPIO_EXP0_1X_REV_09,
            offset: 11,
        },
        flt_24_gpio: None,
        ant_24_gpio: None,
    },
    RadioBusConfig {
        name: "rfb",
        rst_gpio: LinuxGpioConfig { line_name: "PE13" },
        irq_gpio: LinuxGpioConfig { line_name: "PE15" },
        spi: LinuxSpiConfig {
            path: "/dev/spidev3.0",
            max_speed: 12_000_000,
        },
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: GPIO_EXP0_1X_REV_09,
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: GPIO_EXP0_1X_REV_09,
            offset: 1,
        },
        flt_24_gpio: None,
        ant_24_gpio: None,
    },
];

/// The machine named in `/etc/kaonic/kaonic_machine`.
fn read_machine() -> String {
    match std::fs::read_to_string(MACHINE_FILE) {
        Ok(content) => content.trim().to_string(),
        Err(e) => {
            log::warn!(
                "Failed to read {}: {}, using default config",
                MACHINE_FILE,
                e
            );
            "stm32mp1-kaonic-protoa".to_string() // Default fallback
        }
    }
}

/// The line that powers the FPGA.
pub(super) fn fpga_enable_gpio() -> LinuxGpioLineConfig {
    match read_machine().as_str() {
        "stm32mp1-kaonic1x-r09" => LinuxGpioLineConfig {
            chip: GPIO_EXP0_1X_REV_09,
            offset: 7,
        },
        _ => LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 7,
        },
    }
}

/// Selects the radio wiring for the machine named in `/etc/kaonic/kaonic_machine`.
pub(super) fn radio_configs() -> &'static [RadioBusConfig; 2] {
    let machine_config = read_machine();

    log::info!("Kaonic machine configuration: {}", machine_config);

    match machine_config.as_str() {
        "stm32mp1-kaonic-protoa" => &RADIO_CONFIG_REV_A,
        "stm32mp1-kaonic-protob" => &RADIO_CONFIG_REV_B,
        "stm32mp1-kaonic-protoc" => &RADIO_CONFIG_REV_C,
        "stm32mp1-kaonic1s-r24" => &RADIO_CONFIG_REV_24,
        "stm32mp1-kaonic1s-r30" => &RADIO_CONFIG_REV_30,
        "stm32mp1-kaonic1x-r09" => &RADIO_CONFIG_1X_REV_09,
        _ => {
            log::warn!(
                "Unknown machine configuration '{}', using rev_a as default",
                machine_config
            );
            &RADIO_CONFIG_REV_A
        }
    }
}

/// Requests the front-end lines of one radio.
pub(super) fn create_fem(config: &RadioBusConfig) -> Result<Kaonic1SRadioFem, KaonicError> {
    let flt_24 = if let Some(flt_24) = &config.flt_24_gpio {
        LinuxOutputPin::new_from_line(
            flt_24.chip,
            flt_24.offset,
            &format!("{}-flt-sel-24", config.name),
        )
        .ok()
    } else {
        None
    };

    let ant_24 = if let Some(ant_24) = &config.ant_24_gpio {
        LinuxOutputPin::new_from_line(
            ant_24.chip,
            ant_24.offset,
            &format!("{}-ant-sel-24", config.name),
        )
        .ok()
    } else {
        None
    };

    Ok(Kaonic1SRadioFem::new(
        LinuxOutputPin::new_from_line(
            config.flt_v1_gpio.chip,
            config.flt_v1_gpio.offset,
            &format!("{}-flt-sel-v1", config.name),
        )?,
        LinuxOutputPin::new_from_line(
            config.flt_v2_gpio.chip,
            config.flt_v2_gpio.offset,
            &format!("{}-flt-sel-v2", config.name),
        )?,
        flt_24,
        ant_24,
    ))
}

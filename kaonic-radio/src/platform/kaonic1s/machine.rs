use linux_embedded_hal::spidev::SpidevOptions;
use radio_rf215::{
    bus::{BusError, SpiBus},
    modulation::{Modulation, OfdmModulation},
    radio::{AgcGainMap, AuxiliarySettings, FrontendPinConfig, PaVol},
    regs::{BasebandInterrupt, BasebandInterruptMask, RadioInterrupt, RadioInterruptMask},
    Rf215,
};

use crate::platform::{
    kaonic1s::{Kaonic1SRadio, Kaonic1SRadioFem},
    linux::{
        LinuxClock, LinuxGpioConfig, LinuxGpioInterrupt, LinuxGpioLineConfig, LinuxGpioReset,
        LinuxOutputPin, LinuxSpi, LinuxSpiConfig, SharedBus,
    },
};

struct RadioBusConfig {
    name: &'static str,
    rst_gpio: LinuxGpioConfig,
    irq_gpio: LinuxGpioConfig,
    spi: LinuxSpiConfig,
    flt_v1_gpio: LinuxGpioLineConfig,
    flt_v2_gpio: LinuxGpioLineConfig,
    flt_24_gpio: LinuxGpioLineConfig,
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
        flt_24_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 12,
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
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 1,
        },
        flt_24_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip8",
            offset: 2,
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
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 10,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 11,
        },
        flt_24_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 12,
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
        flt_v1_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 0,
        },
        flt_v2_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 1,
        },
        flt_24_gpio: LinuxGpioLineConfig {
            chip: "/dev/gpiochip9",
            offset: 2,
        },
    },
];

const RADIO_CONFIG_REV_C: [RadioBusConfig; 2] = RADIO_CONFIG_REV_B;

pub fn create_radios() -> Result<[Option<Kaonic1SRadio>; 2], BusError> {
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

    log::info!("Kaonic machine configuration: {}", machine_config);

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

    let mut radios: [Option<Kaonic1SRadio>; 2] = [None, None];

    // Create radios based on selected configuration
    for (index, config) in radio_configs.iter().enumerate() {
        match create_radio(config) {
            Ok(radio) => {
                radios[index] = Some(radio);
            }
            Err(_e) => {
                log::error!("failed to create radio {}", config.name);
                // Continue with other radios even if one fails
            }
        }
    }

    Ok(radios)
}

fn create_radio(config: &RadioBusConfig) -> Result<Kaonic1SRadio, BusError> {
    let mut spi = LinuxSpi::open(&config.spi.path).map_err(|_| BusError::ControlFailure)?;

    spi.configure(
        &SpidevOptions::new()
            .max_speed_hz(config.spi.max_speed)
            .build(),
    )
    .map_err(|_| BusError::ControlFailure)?;

    // Create GPIO interfaces
    let reset_gpio = LinuxGpioReset::new(&config.rst_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    let interrupt_gpio = LinuxGpioInterrupt::new(&config.irq_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    // Create clock (system clock)
    let clock = LinuxClock::new();

    // Create the bus with all interfaces
    let bus = SpiBus::new(spi, interrupt_gpio, clock, reset_gpio);

    let bus = std::sync::Arc::new(std::sync::Mutex::new(bus));

    // Probe and initialize the RF215
    let mut radio = Rf215::probe(SharedBus::new(bus), config.name)?;

    // Default configuration for Kaonic1S
    {
        radio
            .trx_09()
            .radio()
            .set_control_pad(FrontendPinConfig::Mode2)
            .map_err(|_| BusError::ControlFailure)?;

        radio
            .trx_09()
            .radio()
            .set_aux_settings(AuxiliarySettings {
                ext_lna_bypass: false,
                aven: false,
                avect: false,
                pavol: PaVol::Voltage2400mV,
                map: AgcGainMap::Extranal12dB,
            })
            .map_err(|_| BusError::ControlFailure)?;

        radio
            .setup_irq(
                RadioInterruptMask::new()
                    .add_irq(RadioInterrupt::TransceiverError)
                    .add_irq(RadioInterrupt::TransceiverReady)
                    .build(),
                BasebandInterruptMask::new()
                    .add_irq(BasebandInterrupt::ReceiverFrameEnd)
                    .add_irq(BasebandInterrupt::TransmitterFrameEnd)
                    .build(),
            )
            .map_err(|_| BusError::ControlFailure)?;

        radio
            .trx_09()
            .configure(&Modulation::Ofdm(OfdmModulation::default()))
            .map_err(|_| BusError::ControlFailure)?;

        radio
            .trx_09()
            .baseband()
            .set_fcs(false)
            .map_err(|_| BusError::ControlFailure)?;

        radio
            .trx_09()
            .radio()
            .receive()
            .map_err(|_| BusError::ControlFailure)?;
    }

    let fem = Kaonic1SRadioFem::new(
        LinuxOutputPin::new_from_line(
            config.flt_v1_gpio.chip,
            config.flt_v1_gpio.offset,
            &format!("{}-flt-sel-v1", config.name),
        )
        .map_err(|_| BusError::ControlFailure)?,
        LinuxOutputPin::new_from_line(
            config.flt_v2_gpio.chip,
            config.flt_v2_gpio.offset,
            &format!("{}-flt-sel-v2", config.name),
        )
        .map_err(|_| BusError::ControlFailure)?,
        LinuxOutputPin::new_from_line(
            config.flt_24_gpio.chip,
            config.flt_24_gpio.offset,
            &format!("{}-flt-sel-24", config.name),
        )
        .map_err(|_| BusError::ControlFailure)?,
    );

    Ok(Kaonic1SRadio::new(radio, fem))
}

use std::sync::{Arc, Mutex};

use linux_embedded_hal::spidev::SpidevOptions;
use radio_common::{modulation::OfdmModulation, Hertz, Modulation, RadioConfigBuilder};
use radio_rf215::{
    bus::{Bus, BusError, SpiBus},
    error::RadioError,
    radio::{AgcGainMap, AuxiliarySettings, FrontendPinConfig, PaVol},
    regs::{BasebandInterrupt, BasebandInterruptMask, RadioInterrupt, RadioInterruptMask},
    transceiver::{Band09, Band24, Transreceiver},
    PadOutputDrive, Rf215,
};

use crate::platform::{
    kaonic1s::{fpga::Kaonic1SFpga, Kaonic1SRadio, Kaonic1SRadioEvent, Kaonic1SRadioFem},
    linux::{
        LinuxClock, LinuxGpioConfig, LinuxGpioInterrupt, LinuxGpioLineConfig, LinuxGpioReset,
        LinuxOutputPin, LinuxSpi, LinuxSpiConfig, SharedBus,
    },
    linux_rf215::{AtomicInterrupt, IrqSignal},
};

struct RadioBusConfig {
    name: &'static str,
    rst_gpio: LinuxGpioConfig,
    irq_gpio: LinuxGpioConfig,
    spi: LinuxSpiConfig,
    flt_v1_gpio: LinuxGpioLineConfig,
    flt_v2_gpio: LinuxGpioLineConfig,
    flt_24_gpio: Option<LinuxGpioLineConfig>,
    ant_24_gpio: Option<LinuxGpioLineConfig>,
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
            // rfb sits on a different SPI controller from rfa and is not
            // reliable at 12 MHz: reads come back subtly corrupted rather than
            // failing, so a frame arrives with a good RSSI and an undecodable
            // LDPC header. Measured on kaonic1s r24, receiving 100 payloads
            // per step: 12 MHz, 10 MHz and 8 MHz all delivered nothing;
            // 6 MHz delivered 100/100 and 283 kbit/s, faster than rfa manages
            // at 12 MHz. Raw frame tests hide this, because they only check
            // the first bytes of a frame while the LDPC header spans ~256.
            //
            // `KAONIC_SPI_HZ_1` overrides this per radio for further testing.
            max_speed: 6_000_000,
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

pub fn create_radios(
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
) -> Result<[Option<Kaonic1SRadio>; 2], BusError> {
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
        "stm32mp1-kaonic1s-r24" => &RADIO_CONFIG_REV_24,
        "stm32mp1-kaonic1s-r30" => &RADIO_CONFIG_REV_30,
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
        match create_radio(index, fpga.clone(), config) {
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

/// Sub-GHz front-end bring-up. The pad mode, external-LNA assumption and AGC
/// gain map can each be overridden per radio through the environment
/// (`KAONIC_RF09_PADFE_1=3`, `KAONIC_RF09_LNABYP_1=1`, `KAONIC_RF09_AGCMAP_1=0`),
/// so a board whose second front-end is wired differently from the first can be
/// probed without a rebuild. Unset, every radio keeps the values that have
/// always shipped.
fn configure_radio_09<I: Bus + Clone>(
    trx: &mut Transreceiver<Band09, I>,
    index: usize,
) -> Result<(), RadioError> {
    let pad = match std::env::var(format!("KAONIC_RF09_PADFE_{index}"))
        .ok()
        .as_deref()
    {
        Some("0") => FrontendPinConfig::Mode0,
        Some("1") => FrontendPinConfig::Mode1,
        Some("3") => FrontendPinConfig::Mode3,
        _ => FrontendPinConfig::Mode2,
    };

    let ext_lna_bypass = matches!(
        std::env::var(format!("KAONIC_RF09_LNABYP_{index}"))
            .ok()
            .as_deref(),
        Some("1")
    );

    let map = match std::env::var(format!("KAONIC_RF09_AGCMAP_{index}"))
        .ok()
        .as_deref()
    {
        Some("0") => AgcGainMap::Internal,
        Some("1") => AgcGainMap::Extranal9dB,
        _ => AgcGainMap::Extranal12dB,
    };

    log::info!(
        "radio[{}] rf09 frontend: padfe={} lna_bypass={} agc_map={}",
        index,
        pad as u8,
        ext_lna_bypass,
        map as u8
    );

    trx.radio()
        .set_control_pad(pad)?
        .set_aux_settings(AuxiliarySettings {
            ext_lna_bypass,
            aven: false,
            avect: false,
            pavol: PaVol::Voltage2400mV,
            map,
        })
        .map_err(|_| BusError::ControlFailure)?;

    trx.baseband().set_fcs(false)?;

    trx.radio().receive()?;

    Ok(())
}

fn configure_radio_24<I: Bus + Clone>(
    trx: &mut Transreceiver<Band24, I>,
) -> Result<(), RadioError> {
    trx.radio()
        .set_control_pad(FrontendPinConfig::Mode3)?
        .set_aux_settings(AuxiliarySettings {
            ext_lna_bypass: true,
            aven: false,
            avect: false,
            pavol: PaVol::Voltage2400mV,
            map: AgcGainMap::Extranal12dB,
        })
        .map_err(|_| BusError::ControlFailure)?;

    trx.baseband().set_fcs(false)?;

    Ok(())
}

fn configure_radio<I: Bus + Clone>(rf: &mut Rf215<I>, index: usize) -> Result<(), RadioError> {
    rf.set_config(&radio_rf215::RfConfig {
        output_drive: PadOutputDrive::Drive8mA,
        irq_active_low: false,
        irq_invert: false,
    })?;

    rf.setup_irq(
        RadioInterruptMask::new()
            .add_irq(RadioInterrupt::TransceiverError)
            .add_irq(RadioInterrupt::TransceiverReady)
            .add_irq(RadioInterrupt::EnergyDetectionCompletion)
            .build(),
        BasebandInterruptMask::new()
            // Frame start, so a detected-but-incomplete frame is
            // distinguishable from one that never arrived.
            .add_irq(BasebandInterrupt::ReceiverFrameStart)
            .add_irq(BasebandInterrupt::ReceiverFrameEnd)
            .add_irq(BasebandInterrupt::TransmitterFrameEnd)
            // Lets the receiver start reading a frame out before it ends.
            .add_irq(BasebandInterrupt::FrameBufferLevelIndication)
            .build(),
    )?;

    rf.set_frame_buffer_level(crate::platform::platform_impl::kaonic1s::rx_stream_level())?;

    configure_radio_09(rf.trx_09(), index)?;
    configure_radio_24(rf.trx_24())?;

    rf.set_frequency(
        &RadioConfigBuilder::new()
            .freq(Hertz::new(869_535_000))
            .channel((index as u16 + 1) * 5)
            .build(),
    )?;

    rf.configure(&Modulation::Ofdm(OfdmModulation {
        tx_power: 18,
        ..Default::default()
    }))?
    .start_receive()?;

    Ok(())
}

fn create_radio(
    index: usize,
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
    config: &RadioBusConfig,
) -> Result<Kaonic1SRadio, BusError> {
    let mut spi = LinuxSpi::open(&config.spi.path)
        .inspect_err(|_| log::error!("can't open spi '{}'", config.spi.path))
        .map_err(|_| BusError::ControlFailure)?;

    // The bus clock can be lowered per radio for bring-up
    // (`KAONIC_SPI_HZ_1=6000000`). The two radios sit on different SPI
    // controllers, so one can be marginal at a clock the other is fine with —
    // and a marginal read corrupts frame data rather than failing outright.
    let max_speed = crate::platform::env::var_or(
        &format!("KAONIC_SPI_HZ_{index}"),
        config.spi.max_speed,
    );

    log::info!(
        "radio[{}] spi {} @ {} Hz",
        index,
        config.spi.path,
        max_speed
    );

    spi.configure(&SpidevOptions::new().max_speed_hz(max_speed).build())
    .inspect_err(|_| log::error!("can't configure spi '{}'", config.spi.path))
    .map_err(|_| BusError::ControlFailure)?;

    // Create GPIO interfaces
    let reset_gpio = LinuxGpioReset::new(&config.rst_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    let interrupt_gpio = LinuxGpioInterrupt::new(&config.irq_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    let irq_signal = Arc::new(IrqSignal::new());

    let radio_event = Kaonic1SRadioEvent::new(irq_signal.clone(), interrupt_gpio);

    let interrupt_atomic = AtomicInterrupt::new(irq_signal.clone());

    // Create clock (system clock)
    let clock = LinuxClock::new();

    // Create the bus with all interfaces
    let bus = SpiBus::new(spi, interrupt_atomic, clock, reset_gpio);

    let bus = std::sync::Arc::new(std::sync::Mutex::new(bus));

    log::debug!(
        "probe radio {} spi:{}@{}Hz",
        config.name,
        config.spi.path,
        config.spi.max_speed
    );

    let mut radio = Rf215::probe(SharedBus::new_with_interrupt(bus, irq_signal), config.name)
        .inspect_err(|_| log::error!("radio probe fail for {}", config.name))?;

    log::debug!("configure radio {}", config.name);

    configure_radio(&mut radio, index)
        .inspect_err(|_| log::error!("radio config fail for {}", config.name))
        .map_err(|_| BusError::ControlFailure)?;

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

    let fem = Kaonic1SRadioFem::new(
        index,
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
        flt_24,
        ant_24,
    );

    Ok(Kaonic1SRadio::new(radio, radio_event, fem, fpga))
}

use std::sync::{Arc, Mutex, atomic::AtomicUsize};

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

use super::{Kaonic1SRadio, Kaonic1SRadioEvent};
use crate::platform::{
    kaonic1s::{
        config::{create_fem, radio_configs, RadioBusConfig},
        fpga::Kaonic1SFpga,
    },
    linux::{LinuxClock, LinuxGpioInterrupt, LinuxGpioReset, LinuxSpi, SharedBus},
    linux_rf215::AtomicInterrupt,
};

pub fn create_radios(
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
) -> Result<[Option<Kaonic1SRadio>; 2], BusError> {
    let radio_configs = radio_configs();

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

fn configure_radio_09<I: Bus + Clone>(
    trx: &mut Transreceiver<Band09, I>,
) -> Result<(), RadioError> {
    trx.radio()
        .set_control_pad(FrontendPinConfig::Mode2)?
        .set_aux_settings(AuxiliarySettings {
            ext_lna_bypass: false,
            aven: false,
            avect: false,
            pavol: PaVol::Voltage2400mV,
            map: AgcGainMap::Extranal12dB,
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
            .add_irq(BasebandInterrupt::ReceiverFrameEnd)
            .add_irq(BasebandInterrupt::TransmitterFrameEnd)
            .build(),
    )?;

    configure_radio_09(rf.trx_09())?;
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

    spi.configure(
        &SpidevOptions::new()
            .max_speed_hz(config.spi.max_speed)
            .build(),
    )
    .inspect_err(|_| log::error!("can't configure spi '{}'", config.spi.path))
    .map_err(|_| BusError::ControlFailure)?;

    // Create GPIO interfaces
    let reset_gpio = LinuxGpioReset::new(&config.rst_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    let interrupt_gpio = LinuxGpioInterrupt::new(&config.irq_gpio.line_name, config.name)
        .map_err(|_| BusError::ControlFailure)?;

    let irq_counter = Arc::new(AtomicUsize::new(0));

    let radio_event = Kaonic1SRadioEvent::new(irq_counter.clone(), interrupt_gpio);

    let interrupt_atomic = AtomicInterrupt::new(irq_counter.clone());

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

    let mut radio = Rf215::probe(SharedBus::new(bus), config.name)
        .inspect_err(|_| log::error!("radio probe fail for {}", config.name))?;

    log::debug!("configure radio {}", config.name);

    configure_radio(&mut radio, index)
        .inspect_err(|_| log::error!("radio config fail for {}", config.name))
        .map_err(|_| BusError::ControlFailure)?;

    let fem = create_fem(config).map_err(|_| BusError::ControlFailure)?;

    Ok(Kaonic1SRadio::new(radio, radio_event, fem, fpga))
}

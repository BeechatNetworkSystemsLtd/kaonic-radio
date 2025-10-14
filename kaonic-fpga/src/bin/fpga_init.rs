use std::time::Duration;

use kaonic_fpga::platform::Kaonic1SFpga;
use kaonic_radio::RadioFem;
use radio_rf215::radio::{RadioFrequency, RadioFrequencyBuilder};

const MAIN_FREQ: RadioFrequency = 869_535_000;

fn main() {
    simple_logger::SimpleLogger::new().env().init().unwrap();

    log::info!("Kaonic FPGA | Init");

    let mut radios = kaonic_radio::platform::create_radios().unwrap();

    let mut radio = radios[0].take().unwrap();

    log::debug!("Configure FEM");
    radio.fem.configure(MAIN_FREQ);

    log::debug!("Enable IQ external loopback");
    radio.radio.set_iq_loopback(true).unwrap();

    log::debug!("Change mode to IQ");
    radio.radio.set_mode(radio_rf215::ChipMode::Radio).unwrap();

    log::debug!("Set Frequency to {} Hz", MAIN_FREQ);
    radio
        .radio
        .set_frequency(&RadioFrequencyBuilder::new().freq(MAIN_FREQ).build())
        .unwrap();

    let mut fpga = Kaonic1SFpga::new().unwrap();

    fpga.enable().expect("fpga enabled");

    loop {
        let write_value = 0xABu8;
        log::debug!("PSRAM:write {}", write_value);
        fpga.write_byte(write_value).unwrap();

        std::thread::sleep(Duration::from_secs(1));

        let read_value = fpga.read_byte().unwrap();
        log::debug!("PSRAM:read {}", read_value);

        std::thread::sleep(Duration::from_secs(1));
    }
}

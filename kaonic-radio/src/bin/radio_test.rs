use std::time::Duration;

use kaonic_radio::platform;
use radio_rf215::{
    bus::Bus,
    radio::{PllLoopBandwidth, RadioFrequencyConfig},
};

fn main() {
    simple_logger::SimpleLogger::new().env().init().unwrap();

    log::info!("Start Radio Test");

    let mut radios = platform::create_radios().unwrap();

    let mut radio = radios[0].take().unwrap();

    let mut rf = radio.rf;
    let bus = &mut radio.bus;

    log::info!("Radio: {} {} {}", rf.part_number(), rf.version(), rf.name());

    rf.trx_09()
        .set_frequency(
            bus,
            &RadioFrequencyConfig {
                freq: 869_535_000,
                channel_spacing: 200_000,
                channel: 10,
                pll_lbw: PllLoopBandwidth::Default,
            },
        )
        .unwrap();

    rf.trx_09().radio().receive(bus).unwrap();
    loop {

        let rssi = rf.trx_09().radio().read_rssi(bus);

        log::trace!("RSSI: {}", rssi.unwrap_or(127));

        bus.delay(Duration::from_millis(10));
    }
}

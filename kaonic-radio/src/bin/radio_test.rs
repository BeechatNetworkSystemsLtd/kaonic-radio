use kaonic_radio::{platform, RadioFem};
use radio_rf215::{
    baseband::BasebandFrame,
    modulation::{Modulation, OfdmModulation},
    radio::{
        AgcGainMap, AuxiliarySettings, FrontendPinConfig, PaVol, PllLoopBandwidth,
        RadioFrequencyConfig,
    },
    regs::{BasebandInterrupt, BasebandInterruptMask, RadioInterrupt, RadioInterruptMask},
};

fn main() {
    simple_logger::SimpleLogger::new().env().init().unwrap();

    log::info!("Start Radio Test");

    let mut radios = platform::create_radios().unwrap();

    let mut radio = radios[0].take().unwrap();

    let mut rf = radio.rf;
    let bus = &mut radio.bus;

    log::info!("Radio: {} {} {}", rf.part_number(), rf.version(), rf.name());

    rf.trx_09().disable_irqs(bus).unwrap();
    rf.trx_24().disable_irqs(bus).unwrap();

    radio.fem.configure(869_535_000);

    rf.trx_09()
        .radio()
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

    rf.trx_09()
        .radio()
        .set_control_pad(bus, FrontendPinConfig::Mode2)
        .unwrap();

    rf.trx_09()
        .radio()
        .set_aux_settings(
            bus,
            AuxiliarySettings {
                ext_lna_bypass: false,
                aven: false,
                avect: false,
                pavol: PaVol::Voltage2400mV,
                map: AgcGainMap::Extranal12dB,
            },
        )
        .unwrap();

    rf.trx_09()
        .configure(bus, &Modulation::Ofdm(OfdmModulation::default()))
        .unwrap();

    rf.trx_09()
        .setup_irq(
            bus,
            RadioInterruptMask::new()
                .add_irq(RadioInterrupt::TransceiverError)
                // .add_irq(RadioInterrupt::TransceiverReady)
                .build(),
            BasebandInterruptMask::new()
                .add_irq(BasebandInterrupt::ReceiverFrameEnd)
                .add_irq(BasebandInterrupt::TransmitterFrameEnd)
                .build(),
        )
        .unwrap();

    let mut frame = BasebandFrame::new();

    // loop {
    //     let tx_frame = BasebandFrame::new_from_slice("HELLO TEST".as_bytes());
    //     log::trace!("Transmit:({}) {}", tx_frame.len(), tx_frame);
    //
    //     rf.trx_09().baseband_transmit(bus, &tx_frame).unwrap();
    //
    //     log::trace!("Wait on transmit finish");
    //     rf.trx_09().baseband().wait_irq(
    //         bus,
    //         BasebandInterrupt::TransmitterFrameEnd,
    //         Duration::from_secs(1),
    //     );
    //
    //     bus.delay(Duration::from_millis(100));
    // }

    log::trace!("Receive");
    rf.trx_09().radio().receive(bus).unwrap();

    loop {
        if rf.trx_09().baseband().wait_irq(
            bus,
            BasebandInterrupt::ReceiverFrameEnd,
            core::time::Duration::from_secs(1),
        ) {
            rf.trx_09().baseband().load_rx(bus, &mut frame).unwrap();
            rf.trx_09().radio().receive(bus).unwrap();
            log::trace!("Frame:({} Bytes)\n\r {}", frame.len(), frame);
        }

        let rssi = rf.trx_09().radio().read_rssi(bus);
        log::trace!("RSSI: {}", rssi.unwrap_or(127));
    }
}

use crate::baseband::{Baseband, BasebandFrame};
use crate::bus::Bus;
use crate::error::RadioError;
use crate::modulation::{self, Modulation};
use crate::radio::{
    Band, FrequencySampleRate, Radio, RadioChannel, RadioFrequency, RadioFrequencyConfig,
    RadioState, RadioTransreceiverConfig, ReceiverBandwidth, RelativeCutOff, TransmitterCutOff,
};
use crate::regs::{self, BasebandInterruptMask, RadioInterruptMask, RegisterAddress};

pub struct Band09;
pub struct Band24;

/// sub-GHz Band
impl Band for Band09 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF09_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC0_BASE_ADDRESS;
    const BASEBAND_FRAME_BUFFER_ADDRESS: RegisterAddress = regs::RG_BBC0_FRAME_BUFFER_ADDRESS;
    const RADIO_IRQ_ADDRESS: RegisterAddress = regs::RG_RF09_IRQS;
    const BASEBAND_IRQ_ADDRESS: RegisterAddress = regs::RG_BBC0_IRQS;
    const MIN_FREQUENCY: RadioFrequency = 389_500_000;
    const MAX_FREQUENCY: RadioFrequency = 1_020_000_000;
    const FREQUENCY_OFFSET: RadioFrequency = 0;
    const MAX_CHANNEL: RadioChannel = 255;
}

impl Band for Band24 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF24_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC1_BASE_ADDRESS;
    const BASEBAND_FRAME_BUFFER_ADDRESS: RegisterAddress = regs::RG_BBC1_FRAME_BUFFER_ADDRESS;
    const RADIO_IRQ_ADDRESS: RegisterAddress = regs::RG_RF24_IRQS;
    const BASEBAND_IRQ_ADDRESS: RegisterAddress = regs::RG_BBC1_IRQS;
    const MIN_FREQUENCY: RadioFrequency = 2_400_000_000;
    const MAX_FREQUENCY: RadioFrequency = 2_483_500_000;
    const FREQUENCY_OFFSET: RadioFrequency = 1_500_000_000;
    const MAX_CHANNEL: RadioChannel = 511;
}

pub struct Transreceiver<B: Band, I: Bus> {
    radio: Radio<B, I>,
    baseband: Baseband<B, I>,
}

impl<B: Band, I: Bus> Transreceiver<B, I> {
    pub(crate) fn new() -> Self {
        Self {
            radio: Radio::<B, I>::new(),
            baseband: Baseband::<B, I>::new(),
        }
    }

    pub fn set_frequency(
        &mut self,
        bus: &mut I,
        config: &RadioFrequencyConfig,
    ) -> Result<(), RadioError> {
        self.radio.change_state(
            bus,
            core::time::Duration::from_millis(100),
            RadioState::TrxOff,
        )?;
        self.radio.set_frequency(bus, config)?;

        Ok(())
    }

    pub fn setup_irq(
        &mut self,
        bus: &mut I,
        radio_irq: RadioInterruptMask,
        baseband_irq: BasebandInterruptMask,
    ) -> Result<(), RadioError> {
        self.radio.setup_irq(bus, radio_irq)?;
        self.baseband.setup_irq(bus, baseband_irq)?;
        Ok(())
    }

    pub fn disable_irqs(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.radio
            .setup_irq(bus, RadioInterruptMask::new().build())?;
        self.baseband
            .setup_irq(bus, BasebandInterruptMask::new().build())?;

        let _ = self.read_irqs(bus)?;

        Ok(())
    }

    pub fn wait_irq(&mut self, bus: &mut I, timeout: core::time::Duration) -> bool {
        bus.wait_interrupt(timeout)
    }

    pub fn read_irqs(
        &mut self,
        bus: &mut I,
    ) -> Result<(RadioInterruptMask, BasebandInterruptMask), RadioError> {
        let rf_irqs = self.radio.read_irqs(bus)?;
        let bb_irqs = self.baseband.read_irqs(bus)?;

        Ok((rf_irqs, bb_irqs))
    }

    pub fn baseband_transmit(
        &mut self,
        bus: &mut I,
        frame: &BasebandFrame,
    ) -> Result<(), RadioError> {
        self.radio.change_state(
            bus,
            core::time::Duration::from_millis(500),
            RadioState::TrxPrep,
        )?;

        self.baseband.load_tx(bus, frame)?;

        self.radio
            .send_command(bus, crate::radio::RadioCommand::Tx)?;

        Ok(())
    }

    pub fn baseband_receive(
        &mut self,
        bus: &mut I,
        frame: &mut BasebandFrame,
    ) -> Result<(), RadioError> {
        self.baseband.load_rx(bus, frame)?;

        Ok(())
    }

    pub fn configure(
        &mut self,
        bus: &mut I,
        modulation: &modulation::Modulation,
    ) -> Result<(), RadioError> {
        self.baseband.disable(bus)?;

        self.baseband.configure(bus, modulation)?;

        self.radio
            .configure_transreceiver(bus, &TransreceiverConfigurator::configure(&modulation))?;

        self.baseband.enable(bus)?;

        Ok(())
    }

    pub fn reset(&mut self, bus: &mut I) -> Result<(), RadioError> {
        self.radio.reset(bus)
    }

    pub fn radio(&mut self) -> &mut Radio<B, I> {
        &mut self.radio
    }

    pub fn baseband(&mut self) -> &mut Baseband<B, I> {
        &mut self.baseband
    }
}

struct TransreceiverConfigurator<B: Band> {
    _band: core::marker::PhantomData<B>,
}

impl TransreceiverConfigurator<Band09> {
    pub fn configure(modulation: &Modulation) -> RadioTransreceiverConfig {
        let mut trx_config = RadioTransreceiverConfig::default();
        let tx_config = &mut trx_config.tx_config;
        let rx_config = &mut trx_config.rx_config;

        match modulation {
            Modulation::Ofdm(ofdm) => {
                // Table 6-90. Recommended Transmitter Frontend Configuration
                // Table 6-93. Recommended PHY Receiver and Digital Frontend Configuration
                match ofdm.opt {
                    modulation::OfdmBandwidthOption::Option1 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut1_000;
                        tx_config.lpfcut = TransmitterCutOff::Flc800kHz;

                        rx_config.rcut = RelativeCutOff::Fcut1_000;
                        rx_config.bw = ReceiverBandwidth::Bw1250kHzIf2000kHz;
                        rx_config.if_shift = true;
                    }
                    modulation::OfdmBandwidthOption::Option2 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc500kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw800kHzIf1000kHz;
                        rx_config.if_shift = true;
                    }
                    modulation::OfdmBandwidthOption::Option3 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc250kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw400kHzIf500kHz;
                        rx_config.if_shift = false;
                    }
                    modulation::OfdmBandwidthOption::Option4 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_500;
                        tx_config.lpfcut = TransmitterCutOff::Flc160kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw250kHzIf250kHz;
                        rx_config.if_shift = true;
                    }
                };

                rx_config.sr = tx_config.sr;
            }
            _ => {}
        }

        trx_config.tx_config.power = 10;

        return trx_config;
    }
}

use crate::baseband::{Baseband, BasebandAutoMode, BasebandFrame};
use crate::bus::Bus;
use crate::error::RadioError;
use crate::modulation::{self, Modulation};
use crate::radio::{
    Band, FrequencySampleRate, PaCur, Radio, RadioChannel, RadioFrequency, RadioFrequencyConfig,
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

pub struct Transreceiver<B: Band, I: Bus + Clone> {
    radio: Radio<B, I>,
    baseband: Baseband<B, I>,
}

impl<B: Band, I: Bus + Clone> Transreceiver<B, I> {
    pub(crate) fn new(bus: I) -> Self {
        let mut trx = Self {
            radio: Radio::<B, I>::new(bus.clone()),
            baseband: Baseband::<B, I>::new(bus.clone()),
        };

        trx.disable_irqs().unwrap();

        trx
    }

    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        self.radio
            .change_state(core::time::Duration::from_millis(100), RadioState::TrxOff)?;

        self.radio.set_frequency(config)?;

        Ok(())
    }

    pub const fn check_band(&self, freq: RadioFrequency) -> bool {
        Radio::<B, I>::check_band(freq)
    }

    pub fn setup_irq(
        &mut self,
        radio_irq: RadioInterruptMask,
        baseband_irq: BasebandInterruptMask,
    ) -> Result<(), RadioError> {
        self.radio.setup_irq(radio_irq)?;
        self.baseband.setup_irq(baseband_irq)?;
        Ok(())
    }

    pub fn disable_irqs(&mut self) -> Result<(), RadioError> {
        self.radio.setup_irq(RadioInterruptMask::new().build())?;
        self.baseband
            .setup_irq(BasebandInterruptMask::new().build())?;

        let _ = self.read_irqs()?;

        Ok(())
    }

    pub fn read_irqs(&mut self) -> Result<(RadioInterruptMask, BasebandInterruptMask), RadioError> {
        let rf_irqs = self.radio.read_irqs()?;
        let bb_irqs = self.baseband.read_irqs()?;

        Ok((rf_irqs, bb_irqs))
    }

    pub fn bb_transmit(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        self.radio
            .change_state(core::time::Duration::from_millis(500), RadioState::TrxPrep)?;

        self.baseband.load_tx(frame)?;

        self.radio.send_command(crate::radio::RadioCommand::Tx)?;

        Ok(())
    }

    pub fn bb_transmit_cca(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        // NOTE: 6.15.5 Clear Channel Assessment with Automatic Transmit (CCATX)

        // NOTE: It is recommended disabling the baseband (set PC.BBEN to 0) to avoid that the
        // baseband decodes/receives any frame during the ED measurement.
        self.baseband.disable()?;

        self.baseband.load_tx(frame)?;

        self.radio
            .change_state(core::time::Duration::from_millis(500), RadioState::Rx)?;

        // NOTE: Do not use procedure CCATX together with procedure Transmit and Switch to Receive (TX2RX)
        self.baseband.set_auto_mode(BasebandAutoMode {
            cca_tx: true,
            auto_rx: false,
            ..Default::default()
        })?;

        self.baseband.set_auto_edt(-50)?;

        self.radio.clear_irqs()?;

        self.radio
            .set_energy_detection(crate::radio::EnergyDetectionMode::Single)?;

        if let Some(irqs) = self.radio.wait_any_irq(
            RadioInterruptMask::new()
                .add_irq(regs::RadioInterrupt::TransceiverReady)
                .add_irq(regs::RadioInterrupt::TransceiverError)
                .build(),
            core::time::Duration::from_millis(100),
        ) {
            if irqs.has_irq(regs::RadioInterrupt::TransceiverError) {
                // NOTE: If the baseband has been disabled for the measurement period and the
                // channel has assessed as busy, the baseband needs to be enabled again by setting
                // PC.BBEN to 1.
                self.baseband.enable()?;
                return Err(RadioError::Timeout);
            }
        }

        Ok(())
    }

    pub fn bb_receive(&mut self, frame: &mut BasebandFrame) -> Result<(), RadioError> {
        self.baseband.load_rx(frame)?;
        Ok(())
    }

    pub fn configure(&mut self, modulation: &modulation::Modulation) -> Result<(), RadioError> {
        self.baseband.disable()?;

        self.baseband.configure(modulation)?;

        self.radio
            .configure_transreceiver(&TransreceiverConfigurator::configure(&modulation))?;

        self.baseband.enable()?;

        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), RadioError> {
        self.radio.reset()
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

        trx_config.tx_config.power = 14;
        trx_config.tx_config.pacur = PaCur::NoReduction;

        return trx_config;
    }
}

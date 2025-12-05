use crate::baseband::{Baseband, BasebandAutoMode, BasebandFrame};
use crate::bus::Bus;
use crate::error::RadioError;
use crate::modulation::{self, Modulation};
use crate::radio::{
    AgcAverageTime, AgcTargetLevel, Band, FrequencySampleRate, PaCur, PaRampTime, Radio,
    RadioChannel, RadioFrequency, RadioFrequencyConfig, RadioState, RadioTransreceiverConfig,
    ReceiverBandwidth, RelativeCutOff, TransmitterCutOff,
};
use crate::regs::{
    self, BasebandInterrupt, BasebandInterruptMask, RadioInterruptMask, RegisterAddress,
};

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

const CHANGE_STATE_DURATION: core::time::Duration = core::time::Duration::from_millis(500);

impl<B: Band, I: Bus + Clone> Transreceiver<B, I> {
    pub(crate) fn new(bus: I) -> Self {
        let trx = Self {
            radio: Radio::<B, I>::new(bus.clone()),
            baseband: Baseband::<B, I>::new(bus.clone()),
        };

        trx
    }

    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxOff)?;

        self.radio.set_frequency(config)?;

        self.radio.receive()?;

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
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxPrep)?;

        self.baseband.load_tx(frame)?;

        self.radio.send_command(crate::radio::RadioCommand::Tx)?;

        Ok(())
    }

    pub fn measure_ed(&mut self) -> Result<i8, RadioError> {
        self.radio
            .set_ed_mode(crate::radio::EnergyDetectionMode::Single)?;

        if let Some(_) = self.radio.wait_irq(
            RadioInterruptMask::new()
                .add_irq(regs::RadioInterrupt::EnergyDetectionCompletion)
                .build(),
            core::time::Duration::from_millis(100),
        ) {
            self.radio.read_edv()
        } else {
            Err(RadioError::Timeout)
        }
    }

    pub fn bb_transmit_cca(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        // NOTE: 6.15.5 Clear Channel Assessment with Automatic Transmit (CCATX)

        // NOTE: It is recommended disabling the baseband (set PC.BBEN to 0) to avoid that the
        // baseband decodes/receives any frame during the ED measurement.
        self.baseband.disable()?;

        self.baseband.load_tx(frame)?;

        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::Rx)?;

        // NOTE: Do not use procedure CCATX together with procedure Transmit and Switch to Receive (TX2RX)
        self.baseband.set_auto_mode(BasebandAutoMode {
            cca_tx: true,
            auto_rx: false,
            ..Default::default()
        })?;

        // TODO: provide EDT value in params
        self.baseband.set_auto_edt(-50)?;

        self.radio.clear_irqs()?;

        self.radio
            .set_ed_mode(crate::radio::EnergyDetectionMode::Single)?;

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

        self.radio.receive()?;

        Ok(())
    }

    pub fn bb_receive(
        &mut self,
        frame: &mut BasebandFrame,
        timeout: core::time::Duration,
    ) -> Result<(), RadioError> {
        self.radio.receive()?;

        if self
            .baseband
            .wait_irq(BasebandInterrupt::ReceiverFrameEnd, timeout)
        {
            self.baseband.load_rx(frame)?;
            Ok(())
        } else {
            Err(RadioError::Timeout)
        }
    }

    pub fn start_receive(&mut self) -> Result<(), RadioError> {
        self.radio.receive()
    }

    pub fn configure(
        &mut self,
        modulation: &modulation::Modulation,
        trx_config: &RadioTransreceiverConfig,
    ) -> Result<(), RadioError> {
        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxOff)?;

        self.baseband.disable()?;

        self.radio.configure_transreceiver(&trx_config)?;

        self.baseband.configure(modulation)?;

        self.baseband.enable()?;

        self.radio.update_frequency()?;

        self.radio.receive()?;

        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), RadioError> {
        self.radio.reset()?;

        self.disable_irqs()?;

        Ok(())
    }

    pub fn radio(&mut self) -> &mut Radio<B, I> {
        &mut self.radio
    }

    pub fn baseband(&mut self) -> &mut Baseband<B, I> {
        &mut self.baseband
    }
}

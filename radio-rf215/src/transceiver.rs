use radio_common::{Hertz, Modulation, RadioChannel, RadioConfig};

use crate::baseband::{Baseband, BasebandAutoMode, BasebandFrame};
use crate::bus::Bus;
use crate::error::RadioError;
use crate::radio::{Band, Radio, RadioState, RadioTransreceiverConfig};
use crate::regs::{
    self, BasebandInterrupt, BasebandInterruptMask, RadioInterruptMask, RegisterAddress,
};

#[derive(Debug)]
pub struct Band09;
#[derive(Debug)]
pub struct Band24;

/// sub-GHz Band
impl Band for Band09 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF09_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC0_BASE_ADDRESS;
    const BASEBAND_FRAME_BUFFER_ADDRESS: RegisterAddress = regs::RG_BBC0_FRAME_BUFFER_ADDRESS;
    const RADIO_IRQ_ADDRESS: RegisterAddress = regs::RG_RF09_IRQS;
    const BASEBAND_IRQ_ADDRESS: RegisterAddress = regs::RG_BBC0_IRQS;
    const MIN_FREQUENCY: Hertz = Hertz::new(389_500_000);
    const MAX_FREQUENCY: Hertz = Hertz::new(1_020_000_000);
    const FREQUENCY_OFFSET: Hertz = Hertz(0);
    const MAX_CHANNEL: RadioChannel = 255;
}

impl Band for Band24 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF24_BASE_ADDRESS;
    const BASEBAND_ADDRESS: RegisterAddress = regs::RG_BBC1_BASE_ADDRESS;
    const BASEBAND_FRAME_BUFFER_ADDRESS: RegisterAddress = regs::RG_BBC1_FRAME_BUFFER_ADDRESS;
    const RADIO_IRQ_ADDRESS: RegisterAddress = regs::RG_RF24_IRQS;
    const BASEBAND_IRQ_ADDRESS: RegisterAddress = regs::RG_BBC1_IRQS;
    const MIN_FREQUENCY: Hertz = Hertz::new(2_400_000_000);
    const MAX_FREQUENCY: Hertz = Hertz::new(2_483_500_000);
    const FREQUENCY_OFFSET: Hertz = Hertz::new(1_500_000_000);
    const MAX_CHANNEL: RadioChannel = 511;
}

#[derive(Debug)]
pub struct Transreceiver<B: Band, I: Bus + Clone> {
    radio: Radio<B, I>,
    baseband: Baseband<B, I>,
    cca_threshold_dbm: i8,
    /// PHY rate of the configured modulation, for sizing transmit waits.
    data_rate_bps: u32,
}

/// Energy-detect threshold used for clear-channel assessment until a caller
/// sets one.
const DEFAULT_CCA_THRESHOLD_DBM: i8 = -50;

const CHANGE_STATE_DURATION: core::time::Duration = core::time::Duration::from_millis(500);

const TX_WAIT_MIN: core::time::Duration = core::time::Duration::from_millis(500);
const TX_WAIT_MAX: core::time::Duration = core::time::Duration::from_secs(8);

impl<B: Band, I: Bus + Clone> Transreceiver<B, I> {
    pub(crate) fn new(bus: I) -> Self {
        let trx = Self {
            radio: Radio::<B, I>::new(bus.clone()),
            baseband: Baseband::<B, I>::new(bus.clone()),
            cca_threshold_dbm: DEFAULT_CCA_THRESHOLD_DBM,
            data_rate_bps: Modulation::Off.data_rate_bps(),
        };

        trx
    }

    pub fn set_frequency(&mut self, config: &RadioConfig) -> Result<(), RadioError> {
        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxOff)?;

        self.radio.set_frequency(config)?;

        self.radio.receive()?;

        Ok(())
    }

    pub fn check_band(&self, freq: Hertz) -> bool {
        Radio::<B, I>::check_band(freq)
    }

    pub fn setup_irq(
        &mut self,
        radio_irq: RadioInterruptMask,
        baseband_irq: BasebandInterruptMask,
    ) -> Result<(), RadioError> {
        self.radio.setup_irq(radio_irq)?;
        self.baseband.setup_irq(baseband_irq)?;

        let _ = self.radio.clear_irqs()?;
        let _ = self.baseband.clear_irqs()?;

        Ok(())
    }

    pub fn disable_irqs(&mut self) -> Result<(), RadioError> {
        self.radio.setup_irq(RadioInterruptMask::new().build())?;
        self.baseband
            .setup_irq(BasebandInterruptMask::new().build())?;

        let _ = self.radio.clear_irqs()?;
        let _ = self.baseband.clear_irqs()?;

        Ok(())
    }

    /// How long a frame needs on air, doubled to cover preamble, PHY header
    /// and CCA. A 2 kB frame is ~7 ms at OFDM and ~2.6 s at the slowest QPSK.
    fn tx_wait(&self, frame_len: usize) -> core::time::Duration {
        let bits = (frame_len as u64).saturating_mul(8);
        let micros = bits.saturating_mul(1_000_000) / u64::from(self.data_rate_bps.max(1));
        core::time::Duration::from_micros(micros.saturating_mul(2)).clamp(TX_WAIT_MIN, TX_WAIT_MAX)
    }

    pub fn bb_transmit(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxPrep)?;

        // Errata #2: TXPREP can be reached with an unlocked PLL.
        self.radio.ensure_pll_lock()?;

        self.baseband.load_tx(frame)?;

        // Both sources: the wait below is on a baseband interrupt, so a stale
        // baseband TXFE would satisfy it immediately.
        self.radio.clear_irqs()?;
        self.baseband.clear_irqs()?;

        self.radio.send_command(crate::radio::RadioCommand::Tx)?;

        // TXFE means the frame finished; TRXRDY only means the state machine
        // reached TX, which is the start of it.
        let finished = self.baseband.wait_irq(
            BasebandInterrupt::TransmitterFrameEnd,
            self.tx_wait(frame.len()),
        );

        if !finished {
            return Err(RadioError::Timeout);
        }

        // An under-run still raises TXFE, but stale buffer content went out.
        if self.baseband.tx_underrun()? {
            return Err(RadioError::IncorrectState);
        }

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

    /// Energy level above which the channel counts as busy for CCA. A
    /// threshold near the strongest expected neighbour (the default) means the
    /// radio almost never defers; one just above the noise floor makes it
    /// share the channel with peers it can actually hear.
    pub fn set_cca_threshold(&mut self, dbm: i8) {
        self.cca_threshold_dbm = dbm;
    }

    pub fn cca_threshold(&self) -> i8 {
        self.cca_threshold_dbm
    }

    pub fn bb_transmit_cca(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        // NOTE: 6.15.5 Clear Channel Assessment with Automatic Transmit (CCATX)

        // NOTE: It is recommended disabling the baseband (set PC.BBEN to 0) to avoid that the
        // baseband decodes/receives any frame during the ED measurement.
        self.baseband.disable()?;

        self.start_receive()?;

        // NOTE: Do not use procedure CCATX together with procedure Transmit and Switch to Receive (TX2RX)
        self.baseband.set_auto_mode(BasebandAutoMode {
            cca_tx: true,
            auto_rx: false,
            ..Default::default()
        })?;

        self.baseband.set_auto_edt(self.cca_threshold_dbm)?;

        self.radio.clear_irqs()?;

        self.radio
            .set_ed_mode(crate::radio::EnergyDetectionMode::Single)?;

        self.baseband.load_tx(frame)?;

        let mut transmitted = false;
        let mut busy = false;

        if let Some(irqs) = self.radio.wait_any_irq(
            RadioInterruptMask::new()
                .add_irq(regs::RadioInterrupt::TransceiverReady)
                .add_irq(regs::RadioInterrupt::TransceiverError)
                .build(),
            core::time::Duration::from_millis(500),
        ) {
            if irqs.has_irq(regs::RadioInterrupt::TransceiverError) {
                // NOTE: If the baseband has been disabled for the measurement period and the
                // channel has assessed as busy, the baseband needs to be enabled again by setting
                // PC.BBEN to 1.
                self.baseband.enable()?;
                busy = true;
            }

            if irqs.has_irq(regs::RadioInterrupt::TransceiverReady) {
                transmitted = true;
            }
        }

        if transmitted {
            Ok(())
        } else if busy {
            // The caller can back off and retry; that is a different situation
            // from the transceiver never answering at all.
            Err(RadioError::ChannelBusy)
        } else {
            Err(RadioError::Timeout)
        }
    }

    pub fn bb_receive(
        &mut self,
        frame: &mut BasebandFrame,
        timeout: core::time::Duration,
    ) -> Result<(), RadioError> {
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

    /// Sets the frame-buffer level that triggers early read-out.
    pub fn set_frame_buffer_level(&mut self, level: u16) -> Result<(), RadioError> {
        self.baseband.set_frame_buffer_level(level)
    }

    /// Receives a frame, reading the first `level` bytes out as soon as they
    /// have arrived instead of waiting for the end of the frame.
    pub fn bb_receive_streaming(
        &mut self,
        frame: &mut BasebandFrame,
        start_timeout: core::time::Duration,
        frame_timeout: core::time::Duration,
        level: u16,
    ) -> Result<(), RadioError> {
        if level == 0 {
            return self.bb_receive(frame, start_timeout);
        }

        let level = level as usize;
        let events = BasebandInterruptMask::new()
            .add_irq(BasebandInterrupt::FrameBufferLevelIndication)
            .add_irq(BasebandInterrupt::ReceiverFrameStart)
            .add_irq(BasebandInterrupt::ReceiverFrameEnd)
            .build();

        // A poll until something arrives, then wait the frame out.
        let mut deadline = self.radio.bus_deadline(start_timeout);
        let mut started = false;
        let mut prefetched = 0usize;

        loop {
            let remaining = self.radio.bus_time_until(deadline);

            if remaining.is_zero() {
                return Err(RadioError::Timeout);
            }

            let Some(irqs) = self.baseband.wait_any_irqs(events, remaining) else {
                return Err(RadioError::Timeout);
            };

            if !started {
                started = true;
                deadline = self.radio.bus_deadline(frame_timeout);
            }

            if irqs.has_irq(BasebandInterrupt::ReceiverFrameEnd) {
                break;
            }

            if prefetched == 0 {
                self.baseband.read_rx_at(0, frame.as_buffer_mut(level))?;
                prefetched = level;
            }
        }

        let len = usize::from(self.baseband.rx_frame_len()?);

        if len > regs::RG_BBCX_FRAME_SIZE {
            return Err(RadioError::IncorrectState);
        }

        let buffer = frame.as_buffer_mut(len);

        if prefetched >= len {
            return Ok(());
        }

        self.baseband
            .read_rx_at(prefetched, &mut buffer[prefetched..])?;

        Ok(())
    }

    /// Re-arms the receiver only if it is not in RX. Reading the state costs
    /// one register read and, unlike an unconditional re-arm, cannot abort a
    /// reception that is already in progress.
    ///
    /// Returns `true` when a re-arm was needed.
    pub fn ensure_receive(&mut self) -> Result<bool, RadioError> {
        match self.radio.read_state()? {
            RadioState::Rx | RadioState::Tx | RadioState::Transition => Ok(false),
            _ => {
                self.radio.receive()?;
                Ok(true)
            }
        }
    }

    pub fn update_irqs(&mut self) -> Result<(), RadioError> {
        self.radio.update_irqs()?;
        self.baseband.update_irqs()?;

        Ok(())
    }

    pub fn configure(
        &mut self,
        modulation: &Modulation,
        trx_config: &RadioTransreceiverConfig,
    ) -> Result<(), RadioError> {
        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxOff)?;

        self.baseband.disable()?;

        self.radio.configure_transreceiver(&trx_config)?;

        self.baseband.configure(modulation)?;
        self.data_rate_bps = modulation.data_rate_bps();

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

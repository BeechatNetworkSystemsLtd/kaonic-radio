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

    pub fn rx_pending(&mut self) -> Result<bool, RadioError> {
        let rx_frame = BasebandInterruptMask::new()
            .add_irq(BasebandInterrupt::ReceiverFrameStart)
            .add_irq(BasebandInterrupt::ReceiverFrameEnd)
            .build();

        Ok(self.baseband.pending_irqs()?.has_any_irqs(&rx_frame))
    }

    pub fn bb_transmit(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        // TXPREP ends whatever the receiver is doing, so the receiver's frame
        // is checked for before the state changes.
        if self.rx_pending()? {
            return Err(RadioError::RxPending);
        }

        self.radio
            .change_state(CHANGE_STATE_DURATION, RadioState::TrxPrep)?;

        // Errata #2: TXPREP can be reached with an unlocked PLL.
        self.radio.ensure_pll_lock()?;

        self.baseband.load_tx(frame)?;

        // The wait below is on a baseband interrupt, so a stale TXFE would
        // satisfy it immediately. Only that one is dropped: a receiver frame
        // end that slipped in before TXPREP still has to reach the read-out.
        self.radio.clear_irqs()?;
        self.baseband.discard_irq(BasebandInterrupt::TransmitterFrameEnd)?;

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

    /// Writes `frame` into the transmit buffer without sending it.
    ///
    /// The write is the slow part of a transmit — 2.6 ms at 6 MHz — and the
    /// buffer is separate from the receive one, so it can go in early: while
    /// the sibling radio is on the air, or before a receive drain. A
    /// [`bb_transmit_cca`](Self::bb_transmit_cca) with `loaded` set then
    /// starts straight at the channel assessment.
    pub fn bb_load_tx(&mut self, frame: &BasebandFrame) -> Result<(), RadioError> {
        self.baseband.load_tx(frame)
    }

    /// Transmits `frame` after a clear channel assessment. With `loaded`
    /// the buffer already holds it, from [`bb_load_tx`](Self::bb_load_tx).
    pub fn bb_transmit_cca(&mut self, frame: &BasebandFrame, loaded: bool) -> Result<(), RadioError> {
        // NOTE: 6.15.5 Clear Channel Assessment with Automatic Transmit (CCATX)

        // The receiver comes first. Disabling the baseband below aborts a
        // frame that is still arriving, and it does so whatever the channel
        // assessment then says — every busy retry used to cut off whatever
        // had started in the meantime. Refusing costs the caller a read-out
        // and a retry; going ahead costs the frame.
        if self.rx_pending()? {
            return Err(RadioError::RxPending);
        }

        // Already in RX when a frame could be pending, so this does not
        // disturb one; it only brings a receiver that was left in TXPREP up
        // for the measurement.
        self.start_receive()?;

        // NOTE: Do not use procedure CCATX together with procedure Transmit and Switch to Receive (TX2RX)
        self.baseband.set_auto_mode(BasebandAutoMode {
            cca_tx: true,
            auto_rx: false,
            ..Default::default()
        })?;

        self.baseband.set_auto_edt(self.cca_threshold_dbm)?;

        // Radio interrupts only: the wait below is on TRXRDY/TRXERR, which a
        // stale one would satisfy at once. Baseband interrupts are left
        // alone, so a receiver frame end that lands from here on is still
        // there for the read-out after the transmit.
        self.radio.clear_irqs()?;

        // NOTE: It is recommended disabling the baseband (set PC.BBEN to 0) to avoid that the
        // baseband decodes/receives any frame during the ED measurement.
        //
        // This is the point of no return for the receiver, which is why it
        // comes after the check above and directly before the measurement
        // that takes the transceiver through TXPREP into TX.
        // The buffer goes in before the measurement starts. CCATX hands the
        // transceiver to the modulator the moment the channel reads idle,
        // and a write still in flight at that point — 2.6 ms at 6 MHz
        // against a 0.96 ms measurement, longer still when the sibling
        // radio's worker holds the core — is overtaken: stale bytes go out
        // behind a valid header, and the far side sees a frame that decodes
        // its header and fails its payload. The transmit buffer is separate
        // from the receive buffer, so loading it disturbs nothing.
        if !loaded {
            self.baseband.load_tx(frame)?;
        }

        self.baseband.disable()?;

        self.radio
            .set_ed_mode(crate::radio::EnergyDetectionMode::Single)?;

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
            // TRXRDY only says the frame started. The under-run flag means
            // something once it has ended, and a frame that under-ran is
            // worth a retry: what went out was not the payload.
            let ended = self
                .baseband
                .wait_irq(BasebandInterrupt::TransmitterFrameEnd, self.tx_wait(frame.len()));
            if ended && self.baseband.tx_underrun()? {
                return Err(RadioError::IncorrectState);
            }
            Ok(())
        } else if busy {
            // The caller can back off and retry; that is a different situation
            // from the transceiver never answering at all.
            Err(RadioError::ChannelBusy)
        } else {
            // Neither outcome re-enabled the baseband, and a receiver left
            // with PC.BBEN=0 decodes nothing until the next successful
            // transmit turns it back on.
            let _ = self.baseband.enable();
            Err(RadioError::Timeout)
        }
    }

    /// Receives a frame: a poll of `start_timeout` for one to begin, and
    /// once one has begun, `frame_timeout` for it to end.
    ///
    /// The two are separate because the caller keeps the poll short so a
    /// transmit is not starved, and a frame takes longer than that to
    /// arrive. Waiting only the poll out used to return `Timeout` with the
    /// frame still on the air, and the caller then went off to transmit
    /// over it.
    pub fn bb_receive(
        &mut self,
        frame: &mut BasebandFrame,
        start_timeout: core::time::Duration,
        frame_timeout: core::time::Duration,
    ) -> Result<(), RadioError> {
        let events = BasebandInterruptMask::new()
            .add_irq(BasebandInterrupt::ReceiverFrameStart)
            .add_irq(BasebandInterrupt::ReceiverFrameEnd)
            .build();

        let mut deadline = self.radio.bus_deadline(start_timeout);
        let mut started = false;

        loop {
            let remaining = self.radio.bus_time_until(deadline);

            if remaining.is_zero() {
                return Err(RadioError::Timeout);
            }

            let Some(irqs) = self.baseband.wait_any_irqs(events, remaining) else {
                return Err(RadioError::Timeout);
            };

            if irqs.has_irq(BasebandInterrupt::ReceiverFrameEnd) {
                break;
            }

            // Extended once, as in the streaming path: a start with no end
            // inside the budget was a false start, and must not be allowed
            // to renew itself.
            if !started {
                started = true;
                deadline = self.radio.bus_deadline(frame_timeout);
            }
        }

        // Freeze the buffer before reading it. The chip stays in RX
        // after a frame ends, and a frame that starts arriving during a
        // delayed read-out overwrites the one being read: the reader
        // gets a corrupt frame rather than a missed one. Stepping to
        // TXPREP first turns that into a plain miss, which the coding
        // above already handles. Receive is re-armed by the caller.
        let _ = self.radio.stop_receive();
        self.baseband.load_rx(frame)?;

        Ok(())
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
            return self.bb_receive(frame, start_timeout, frame_timeout);
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
                // As in `bb_receive`: freeze the buffer before the tail read.
                let _ = self.radio.stop_receive();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusError;
    use crate::radio::RadioCommand;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;

    const BBEN: u8 = 0b0000_0100;
    const RXFS: u8 = BasebandInterrupt::ReceiverFrameStart as u8;
    const RXFE: u8 = BasebandInterrupt::ReceiverFrameEnd as u8;

    const PC: RegisterAddress = Band09::BASEBAND_ADDRESS + regs::RG_BBCX_PC;
    const RXFLL: RegisterAddress = Band09::BASEBAND_ADDRESS + regs::RG_BBCX_RXFLL;
    const CMD: RegisterAddress = Band09::RADIO_ADDRESS + regs::RG_RFXX_CMD;
    const STATE: RegisterAddress = Band09::RADIO_ADDRESS + regs::RG_RFXX_STATE;
    const PLL: RegisterAddress = Band09::RADIO_ADDRESS + regs::RG_RFXX_PLL;
    const EDC: RegisterAddress = Band09::RADIO_ADDRESS + regs::RG_RFXX_EDC;

    /// Registers, plus a script of baseband interrupt statuses handed out
    /// one per poll (clear-on-read, like the chip), and a clock that only
    /// moves while something waits — so a test can tell a wait that was
    /// bounded from one that spun. A command is mirrored into the state
    /// register at once, and the PLL reads as locked, so the state machine
    /// in `Radio` settles instead of polling for ever.
    #[derive(Default)]
    struct Chip {
        regs: HashMap<RegisterAddress, u8>,
        writes: Vec<(RegisterAddress, u8)>,
        bb_irqs: VecDeque<u8>,
        now_us: u64,
    }

    #[derive(Clone)]
    struct MockBus(Rc<RefCell<Chip>>);

    impl MockBus {
        fn new(bb_irqs: &[u8]) -> Self {
            let mut chip = Chip::default();
            chip.regs.insert(PC, BBEN);
            chip.regs.insert(STATE, RadioState::Rx as u8);
            chip.regs.insert(PLL, 0b0000_0010);
            chip.bb_irqs = bb_irqs.iter().copied().collect();
            Self(Rc::new(RefCell::new(chip)))
        }

        fn reg(&self, addr: RegisterAddress) -> u8 {
            *self.0.borrow().regs.get(&addr).unwrap_or(&0)
        }

        fn wrote(&self, addr: RegisterAddress) -> bool {
            self.0.borrow().writes.iter().any(|(a, _)| *a == addr)
        }

        fn wrote_value(&self, addr: RegisterAddress, value: u8) -> bool {
            self.0
                .borrow()
                .writes
                .iter()
                .any(|(a, v)| *a == addr && *v == value)
        }

        fn elapsed_ms(&self) -> u64 {
            self.0.borrow().now_us / 1000
        }
    }

    impl Bus for MockBus {
        fn write_regs(
            &mut self,
            addr: RegisterAddress,
            values: &[regs::RegisterValue],
        ) -> Result<(), BusError> {
            let mut chip = self.0.borrow_mut();
            for (i, value) in values.iter().enumerate() {
                let addr = addr + i as RegisterAddress;
                chip.regs.insert(addr, *value);
                chip.writes.push((addr, *value));

                if addr == CMD {
                    chip.regs.insert(STATE, *value);
                }
            }
            Ok(())
        }

        fn read_regs(
            &mut self,
            addr: RegisterAddress,
            values: &mut [regs::RegisterValue],
        ) -> Result<(), BusError> {
            let mut chip = self.0.borrow_mut();
            for (i, value) in values.iter_mut().enumerate() {
                let addr = addr + i as RegisterAddress;
                *value = if addr == Band09::BASEBAND_IRQ_ADDRESS {
                    chip.bb_irqs.pop_front().unwrap_or(0)
                } else {
                    *chip.regs.get(&addr).unwrap_or(&0)
                };
            }
            Ok(())
        }

        fn wait_interrupt(&mut self, timeout: Option<core::time::Duration>) -> bool {
            let step = timeout.map(|t| t.as_micros() as u64).unwrap_or(1_000).max(1);
            self.0.borrow_mut().now_us += step;
            false
        }

        fn delay(&mut self, timeout: core::time::Duration) {
            self.0.borrow_mut().now_us += (timeout.as_micros() as u64).max(1);
        }

        fn current_time(&mut self) -> u64 {
            self.0.borrow().now_us / 1000
        }

        fn hardware_reset(&mut self) -> Result<(), BusError> {
            Ok(())
        }
    }


    fn ms(value: u64) -> core::time::Duration {
        core::time::Duration::from_millis(value)
    }

    #[test]
    fn transmit_yields_to_an_unread_frame_and_leaves_it_for_the_read_out() {
        let bus = MockBus::new(&[RXFE]);
        let mut trx = Transreceiver::<Band09, MockBus>::new(bus.clone());

        // A frame ended and nobody read it: the transmit must not start, and
        // above all must not touch the receiver on the way out.
        assert_eq!(
            trx.bb_transmit_cca(&BasebandFrame::new_from_slice(&[1, 2, 3]), false),
            Err(RadioError::RxPending)
        );
        assert_eq!(bus.reg(PC) & BBEN, BBEN);
        assert!(!bus.wrote(CMD));
        assert!(!bus.wrote(EDC));

        // The check claimed nothing: the read-out that follows still sees
        // the same frame end, without another interrupt from the chip.
        {
            let mut chip = bus.0.borrow_mut();
            chip.regs.insert(RXFLL, 2);
            chip.regs.insert(Band09::BASEBAND_FRAME_BUFFER_ADDRESS, 0xAB);
            chip.regs.insert(Band09::BASEBAND_FRAME_BUFFER_ADDRESS + 1, 0xCD);
        }

        let mut frame = BasebandFrame::new();
        assert_eq!(trx.bb_receive(&mut frame, ms(1), ms(10)), Ok(()));
        assert_eq!(frame.as_slice(), &[0xAB, 0xCD]);
        assert_eq!(bus.reg(CMD), RadioCommand::TrxPrep as u8);
    }

    #[test]
    fn transmit_yields_to_a_frame_still_on_the_air() {
        let bus = MockBus::new(&[RXFS]);
        let mut trx = Transreceiver::<Band09, MockBus>::new(bus.clone());

        assert_eq!(
            trx.bb_transmit_cca(&BasebandFrame::new_from_slice(&[1]), false),
            Err(RadioError::RxPending)
        );
        assert_eq!(bus.reg(PC) & BBEN, BBEN);
        assert!(!bus.wrote(EDC));
    }

    #[test]
    fn receive_waits_a_started_frame_out_past_the_poll() {
        // Start inside the 1 ms poll, end 6 ms later: only the frame budget
        // covers it.
        let bus = MockBus::new(&[RXFS, 0, 0, RXFE]);
        let mut trx = Transreceiver::<Band09, MockBus>::new(bus.clone());

        let mut frame = BasebandFrame::new();
        assert_eq!(trx.bb_receive(&mut frame, ms(1), ms(100)), Ok(()));
        assert!(bus.elapsed_ms() >= 4);
        assert_eq!(bus.reg(CMD), RadioCommand::TrxPrep as u8);
    }

    #[test]
    fn receive_gives_up_on_a_false_start_within_the_budget() {
        let bus = MockBus::new(&[RXFS]);
        let mut trx = Transreceiver::<Band09, MockBus>::new(bus.clone());

        let mut frame = BasebandFrame::new();
        assert_eq!(
            trx.bb_receive(&mut frame, ms(1), ms(10)),
            Err(RadioError::Timeout)
        );

        // Bounded by the budget, not the poll, and not renewed.
        let waited = bus.elapsed_ms();
        assert!(waited >= 10 && waited < 20, "waited {} ms", waited);

        // The start is consumed with the timeout, so the next transmit is
        // not held back by it.
        assert_eq!(trx.rx_pending(), Ok(false));
    }

    #[test]
    fn transmit_proceeds_when_nothing_is_pending() {
        let bus = MockBus::new(&[0]);
        let mut trx = Transreceiver::<Band09, MockBus>::new(bus.clone());

        assert_eq!(trx.rx_pending(), Ok(false));

        // No pending frame: the CCA procedure runs, which means the
        // measurement is started with the baseband off. The mock never
        // answers it, so the procedure times out — and hands the baseband
        // back rather than leaving the receiver deaf.
        assert_eq!(
            trx.bb_transmit_cca(&BasebandFrame::new_from_slice(&[1]), false),
            Err(RadioError::Timeout)
        );
        assert!(bus.wrote(EDC));
        assert!(bus.wrote_value(PC, 0));
        assert_eq!(bus.reg(PC) & BBEN, BBEN);
    }
}

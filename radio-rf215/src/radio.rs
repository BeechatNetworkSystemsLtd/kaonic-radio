use core::marker::PhantomData;

use crate::{
    bus::Bus,
    error::RadioError,
    regs::{self, RadioInterruptMask, RegisterAddress},
};

/// Frequency in Hz
pub type RadioFrequency = u32;
pub type RadioChannel = u16;

#[derive(PartialEq, Clone, Copy)]
pub struct RadioFrequencyConfig {
    pub freq: RadioFrequency,
    pub channel_spacing: RadioFrequency,
    pub channel: RadioChannel,
    pub pll_lbw: PllLoopBandwidth,
}

pub struct RadioFrequencyBuilder {
    config: RadioFrequencyConfig,
}

impl RadioFrequencyBuilder {
    pub const fn new() -> Self {
        Self {
            config: RadioFrequencyConfig {
                freq: 869_535_000,
                channel_spacing: 200_000,
                channel: 10,
                pll_lbw: PllLoopBandwidth::Default,
            },
        }
    }

    pub fn freq(mut self, freq: RadioFrequency) -> Self {
        self.config.freq = freq;
        self
    }

    pub fn channel(mut self, channel: RadioChannel) -> Self {
        self.config.channel = channel;
        self
    }

    pub fn channel_spacing(mut self, spacing: RadioFrequency) -> Self {
        self.config.channel_spacing = spacing;
        self
    }

    pub fn build(self) -> RadioFrequencyConfig {
        self.config
    }
}

pub trait Band {
    const RADIO_ADDRESS: RegisterAddress;
    const BASEBAND_ADDRESS: RegisterAddress;
    const BASEBAND_FRAME_BUFFER_ADDRESS: RegisterAddress;
    const RADIO_IRQ_ADDRESS: RegisterAddress;
    const BASEBAND_IRQ_ADDRESS: RegisterAddress;
    const MIN_FREQUENCY: RadioFrequency;
    const MAX_FREQUENCY: RadioFrequency;
    const FREQUENCY_OFFSET: RadioFrequency;
    const MAX_CHANNEL: RadioChannel;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum FrontendPinConfig {
    Mode0 = 0x00, // no Frontend control; FEAnn and FEBnn output is always 0
    Mode1 = 0x01, // (1 pin is TX switch; 1 pin is RX switch; LNA can be bypassed)
    Mode2 = 0x02, // (1 pin is enable, 1 pin is TXRX switch; 1 | 0 additional option)
    Mode3 = 0x03, // (1 pin is TXRX switch, 1 pin is LNA Bypass, 1 pin (MCU) is enable)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum EnergyDetectionMode {
    Auto = 0x00,
    Single = 0x01,
    Continuous = 0x02,
    Off = 0x03,
}

pub struct AuxiliarySettings {
    pub ext_lna_bypass: bool, // External LNA Bypass Availability
    pub aven: bool,           // Analog Voltage Enable
    pub avect: bool,          // Analog Voltage External Driven
    pub pavol: PaVol,         //Power Amplifier Voltage Control
    pub map: AgcGainMap,
}

impl Default for AuxiliarySettings {
    fn default() -> Self {
        Self {
            ext_lna_bypass: false,
            aven: false,
            avect: false,
            pavol: PaVol::Voltage2400mV,
            map: AgcGainMap::Internal,
        }
    }
}

/// AGC Average Time in Number of Samples
/// The time of averaging RX data samples for the AGC values is defined by number of samples
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum AgcAverageTime {
    Samples8 = 0x00,
    Samples16 = 0x01,
    Samples32 = 0x02,
    Samples64 = 0x03,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum AgcGainMap {
    Internal = 0x00,
    Extranal9dB = 0x01,
    Extranal12dB = 0x02,
}

/// AGC Target Level
/// the AGC target level relative to ADC full scale.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum AgcTargetLevel {
    TargetN21dB = 0x00,
    TargetN24dB = 0x01,
    TargetN27dB = 0x02,
    TargetN30dB = 0x03,
    TargetN33dB = 0x04,
    TargetN36dB = 0x05,
    TargetN39dB = 0x06,
    TargetN42dB = 0x07,
}

pub struct AgcReceiverGain {
    pub target_level: AgcTargetLevel,
    pub gcw: u8,
}

impl Default for AgcReceiverGain {
    fn default() -> Self {
        Self {
            target_level: AgcTargetLevel::TargetN30dB,
            gcw: 23,
        }
    }
}

// 6.2.5.3 RFn_AGCC – Receiver AGC Control 0
pub struct AgcReceiverControl {
    pub agc_input: bool,              // This bit controls the input signal of the AGC
    pub average_time: AgcAverageTime, // The time of averaging RX data samples for the AGC values is defined by number of samples
    pub reset: bool,                  // AGC Reset
    pub freeze_control: bool,         // AGC Freeze Control
    pub enabled: bool,                // AGC Enable
}

impl Default for AgcReceiverControl {
    fn default() -> Self {
        Self {
            agc_input: false,
            average_time: AgcAverageTime::Samples8,
            reset: false,
            freeze_control: false,
            enabled: true,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum FrequencySampleRate {
    SampleRate4000kHz = 0x01,
    SampleRate2000kHz = 0x02,
    SampleRate1333kHz = 0x03,
    SampleRate1000kHz = 0x04,
    SampleRate800kHz = 0x05,
    SampleRate666kHz = 0x06,
    SampleRate500kHz = 0x08,
    SampleRate400kHz = 0x0A,
}

/// Filter relative cut-off frequency
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RelativeCutOff {
    Fcut0_250 = 0x00,
    Fcut0_375 = 0x01,
    Fcut0_500 = 0x02,
    Fcut0_750 = 0x03,
    Fcut1_000 = 0x04,
}

/// Transmitter low pass filter cut-off frequency
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum TransmitterCutOff {
    Flc80kHz = 0x00,
    Flc100kHz = 0x01,
    Flc125kHz = 0x02,
    Flc160kHz = 0x03,
    Flc200kHz = 0x04,
    Flc250kHz = 0x05,
    Flc315kHz = 0x06,
    Flc400kHz = 0x07,
    Flc500kHz = 0x08,
    Flc625kHz = 0x09,
    Flc800kHz = 0x0A,
    Flc1000kHz = 0x0B,
}

/// Power Amplifier Ramp Time
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PaRampTime {
    Paramp4 = 0x00,
    Paramp8 = 0x01,
    Paramp16 = 0x02,
    Paramp32 = 0x03,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum ReceiverBandwidth {
    Bw160kHzIf250kHz = 0x0,   // fBW=160kHz; fIF=250kHz
    Bw200kHzIf250kHz = 0x1,   // fBW=200kHz; fIF=250kHz
    Bw250kHzIf250kHz = 0x2,   // fBW=250kHz; fIF=250kHz
    Bw320kHzIf500kHz = 0x3,   // fBW=320kHz; fIF=500kHz
    Bw400kHzIf500kHz = 0x4,   // fBW=400kHz; fIF=500kHz
    Bw500kHzIf500kHz = 0x5,   // fBW=500kHz; fIF=500kHz
    Bw630kHzIf1000kHz = 0x6,  //  fBW=630kHz; fIF=1000kHz
    Bw800kHzIf1000kHz = 0x7,  // fBW=800kHz; fIF=1000kHz
    Bw1000kHzIf1000kHz = 0x8, // fBW=1000kHz; fIF=1000kHz
    Bw1250kHzIf2000kHz = 0x9, //fBW=1250kHz; fIF=2000kHz
    Bw1600kHzIf2000kHz = 0xA, //fBW=1600kHz; fIF=2000kHz
    Bw2000kHzIf2000kHz = 0xB, // fBW=2000kHz; fIF=2000kHz
}

/// Transmitter Frontend Configuration
pub struct RadioTransmitterConfig {
    pub sr: FrequencySampleRate,
    pub rcut: RelativeCutOff,
    pub lpfcut: TransmitterCutOff,
    pub paramp: PaRampTime,
    pub pacur: PaCur,
    pub power: u8,
}

impl Default for RadioTransmitterConfig {
    fn default() -> Self {
        Self {
            sr: FrequencySampleRate::SampleRate4000kHz,
            rcut: RelativeCutOff::Fcut0_250,
            lpfcut: TransmitterCutOff::Flc500kHz,
            paramp: PaRampTime::Paramp4,
            pacur: PaCur::NoReduction,
            power: 1,
        }
    }
}

/// Receiver Frontend Configuration
pub struct RadioReceiverConfig {
    pub sr: FrequencySampleRate,
    pub rcut: RelativeCutOff,
    pub bw: ReceiverBandwidth,
    pub if_inversion: bool,
    pub if_shift: bool,
}

impl Default for RadioReceiverConfig {
    fn default() -> Self {
        Self {
            sr: FrequencySampleRate::SampleRate4000kHz,
            rcut: RelativeCutOff::Fcut0_250,
            bw: ReceiverBandwidth::Bw2000kHzIf2000kHz,
            if_inversion: false,
            if_shift: false,
        }
    }
}

pub struct RadioTransreceiverConfig {
    pub tx_config: RadioTransmitterConfig,
    pub rx_config: RadioReceiverConfig,
    pub agc_control: AgcReceiverControl,
    pub agc_gain: AgcReceiverGain,
    pub edd: core::time::Duration,
}

impl Default for RadioTransreceiverConfig {
    fn default() -> Self {
        Self {
            tx_config: Default::default(),
            rx_config: Default::default(),
            agc_control: Default::default(),
            agc_gain: Default::default(),
            edd: core::time::Duration::from_micros(128),
        }
    }
}

/// Power Amplifier Voltage
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PaVol {
    Voltage2000mV = 0x00,
    Voltage2200mV = 0x01,
    Voltage2400mV = 0x02,
}

/// Power amplifier current
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PaCur {
    Reduction22mA = 0x00, // 3dB reduction of max. small signal gain
    Reduction18mA = 0x01, // 2dB reduction of max. small signal gain
    Reduction11mA = 0x02, // 1dB reduction of max. small signal gain
    NoReduction = 0x03,   // max. transmit small signal gain
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PllLoopBandwidth {
    Default = 0x00 << 4,
    Smaller = 0x01 << 4, // 15% smaller PLL loopbandwidth
    Larger = 0x02 << 4,  // 15% larger PLL loopbandwidth
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RadioState {
    PowerOff = 0x00,
    Sleep = 0x01,
    TrxOff = 0x02,
    TrxPrep = 0x03,
    Tx = 0x04,
    Rx = 0x05,
    Transition = 0x06,
    Reset = 0x07,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RadioCommand {
    Nop = 0x0,
    Sleep = 0x1,
    TrxOff = 0x2,
    TrxPrep = 0x3,
    Tx = 0x4,
    Rx = 0x5,
    Reset = 0x7,
}

/// Represents radio module part of the transceiver
/// B is a sub-GHz or 2.4GHz band
pub struct Radio<B, I>
where
    B: Band,
    I: Bus,
{
    _band: PhantomData<B>,
    bus: I,
}

impl<B, I> Radio<B, I>
where
    B: Band,
    I: Bus,
{
    pub fn new(bus: I) -> Self {
        Self {
            _band: PhantomData::default(),
            bus,
        }
    }

    pub fn send_command(&mut self, command: RadioCommand) -> Result<(), RadioError> {
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_CMD), command as u8)
            .map_err(|e| e.into())
    }

    /// Requests transition into a 'state'
    pub fn set_state(&mut self, state: RadioState) -> Result<(), RadioError> {
        let command = match state {
            RadioState::PowerOff => RadioCommand::Nop,
            RadioState::Sleep => RadioCommand::Sleep,
            RadioState::TrxOff => RadioCommand::TrxOff,
            RadioState::TrxPrep => RadioCommand::TrxPrep,
            RadioState::Tx => RadioCommand::Tx,
            RadioState::Rx => RadioCommand::Rx,
            RadioState::Reset => RadioCommand::Reset,
            RadioState::Transition => return Err(RadioError::IncorrectState),
        };

        self.send_command(command)
    }

    pub fn setup_irq(&mut self, irq_mask: RadioInterruptMask) -> Result<(), RadioError> {
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_IRQM), irq_mask.get())?;
        Ok(())
    }

    pub fn wait_on_state<F: Fn(RadioState) -> bool>(
        &mut self,
        timeout: core::time::Duration,
        check_state: F,
    ) -> Result<RadioState, RadioError> {
        let deadline = (self.bus.current_time() as u128) + timeout.as_millis();

        loop {
            let state = self.read_state()?;

            if check_state(state) {
                return Ok(state);
            }

            if (self.bus.current_time() as u128) > deadline {
                return Err(RadioError::CommunicationFailure);
            }

            self.bus.delay(core::time::Duration::from_micros(100));
        }
    }

    pub fn change_state(
        &mut self,
        timeout: core::time::Duration,
        state: RadioState,
    ) -> Result<RadioState, RadioError> {
        self.set_state(state)?;

        self.wait_on_state(timeout, |s| s == state)
    }

    pub fn read_state(&mut self) -> Result<RadioState, RadioError> {
        let state_value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_STATE))?;

        let state = match state_value {
            0x00 => RadioState::PowerOff,
            0x01 => RadioState::Sleep,
            0x02 => RadioState::TrxOff,
            0x03 => RadioState::TrxPrep,
            0x04 => RadioState::Tx,
            0x05 => RadioState::Rx,
            0x06 => RadioState::Transition,
            0x07 => RadioState::Reset,
            _ => return Err(RadioError::IncorrectState),
        };

        Ok(state)
    }

    pub fn wait_interrupt(&mut self, timeout: core::time::Duration) -> bool {
        self.bus.wait_interrupt(timeout)
    }

    pub fn receive(&mut self) -> Result<(), RadioError> {
        loop {
            let state = self.wait_on_state(core::time::Duration::from_millis(100), |s| {
                (s == RadioState::TrxOff) || (s == RadioState::TrxPrep)
            });

            let mut should_change_state = false;
            if let Err(_) = state {
                should_change_state = true;
            } else if let Ok(state) = state {
                should_change_state = state != RadioState::TrxPrep;
            }

            if should_change_state {
                self.set_state(RadioState::TrxPrep)?;
            } else {
                break;
            }
        }

        self.set_state(RadioState::Rx)?;

        self.wait_on_state(core::time::Duration::from_millis(100), |s| {
            s == RadioState::Rx
        })?;

        Ok(())
    }

    /// Configures Radio for a specific frequency, spacing and channel
    pub fn set_frequency(&mut self, config: &RadioFrequencyConfig) -> Result<(), RadioError> {
        if config.freq < B::MIN_FREQUENCY
            || config.freq > B::MAX_FREQUENCY
            || config.freq < B::FREQUENCY_OFFSET
        {
            return Err(RadioError::IncorrectConfig);
        }

        if config.channel > B::MAX_CHANNEL {
            return Err(RadioError::IncorrectConfig);
        }

        let cs = config.channel_spacing / regs::RG_RFXX_FREQ_RESOLUTION_HZ;
        if cs > 0xFF {
            return Err(RadioError::IncorrectConfig);
        }

        let freq = (config.freq - B::FREQUENCY_OFFSET) / regs::RG_RFXX_FREQ_RESOLUTION_HZ;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_CS), cs as u8)?;

        self.bus
            .write_reg_u16(Self::abs_reg(regs::RG_RFXX_CCF0L), freq as u16)?;

        let channel = config.channel.to_le_bytes();

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_CNL), channel[0])?;

        // Using IEEE-compliant Scheme
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_CNM), 0x00 | channel[1])?;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_PLL), config.pll_lbw as u8)?;

        Ok(())
    }

    pub fn update_frequency(&mut self) -> Result<(), RadioError> {
        self.bus
            .modify_reg_u8(Self::abs_reg(regs::RG_RFXX_CNM), 0x00, 0x00)?;

        Ok(())
    }

    pub fn read_rssi(&mut self) -> Result<i8, RadioError> {
        let value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_RSSI))?;
        let rssi = value as i8;

        if rssi == 127 {
            return Err(RadioError::IncorrectState);
        }

        Ok(rssi)
    }

    pub fn read_edv(&mut self) -> Result<i8, RadioError> {
        let value = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_EDV))?;
        let edv = value as i8;

        Ok(edv)
    }

    pub fn set_ed_mode(&mut self, mode: EnergyDetectionMode) -> Result<(), RadioError> {
        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_EDC), mode as u8)?;

        Ok(())
    }

    pub fn set_ed_duration(&mut self, duration: core::time::Duration) -> Result<(), RadioError> {
        let dtb_mul: [u32; 4] = [2, 8, 32, 128];

        let expected_duration = duration.as_micros() as u32;
        for i in 0..dtb_mul.len() {
            let df = expected_duration / dtb_mul[i];
            if df < 63 {
                let edd = ((df as u8) << 2) | (i as u8);

                self.bus
                    .write_reg_u8(Self::abs_reg(regs::RG_RFXX_EDD), edd)?;

                return Ok(());
            }
        }

        Err(RadioError::IncorrectConfig)
    }

    pub fn wait_irq(
        &mut self,
        irq_mask: RadioInterruptMask,
        timeout: core::time::Duration,
    ) -> Option<RadioInterruptMask> {
        let deadline = self.bus.deadline(timeout);

        loop {
            if self.bus.deadline_reached(deadline) {
                break;
            }

            if self
                .bus
                .wait_interrupt(core::time::Duration::from_micros(500))
            {
                if let Ok(irqs) = self.read_irqs() {
                    if irqs.has_irqs(irq_mask) {
                        return Some(irqs);
                    }
                }
            }
        }

        return None;
    }

    pub fn wait_any_irq(
        &mut self,
        irq_mask: RadioInterruptMask,
        timeout: core::time::Duration,
    ) -> Option<RadioInterruptMask> {
        let deadline = self.bus.deadline(timeout);

        loop {
            if self.bus.deadline_reached(deadline) {
                break;
            }

            if self
                .bus
                .wait_interrupt(core::time::Duration::from_micros(500))
            {
                if let Ok(irqs) = self.read_irqs() {
                    if irqs.has_any_irqs(irq_mask) {
                        return Some(irqs);
                    }
                }
            }
        }

        return None;
    }

    pub fn read_irqs(&mut self) -> Result<RadioInterruptMask, RadioError> {
        let irq_status = self.bus.read_reg_u8(B::RADIO_IRQ_ADDRESS)?;
        Ok(RadioInterruptMask::new_from_mask(irq_status))
    }

    pub fn clear_irqs(&mut self) -> Result<(), RadioError> {
        let _ = self.read_irqs()?;
        Ok(())
    }

    pub fn configure_transmitter(
        &mut self,
        config: &RadioTransmitterConfig,
    ) -> Result<(), RadioError> {
        // Transmitter TX Digital Frontend
        {
            let mut txdfe = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_TXDFE))?;

            // Clear SR and RCUT bits
            txdfe = txdfe & 0b0001_0000;
            txdfe = txdfe | (config.sr as u8) | ((config.rcut as u8) << 5);

            self.bus
                .write_reg_u8(Self::abs_reg(regs::RG_RFXX_TXDFE), txdfe)?;
        }

        // Transmitter Filter Cutoff Control and PA Ramp Time
        {
            let mut txcutc = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_TXCUTC))?;

            // Clear SR and RCUT bits
            txcutc = txcutc & 0b0011_0000;
            txcutc = txcutc | (config.lpfcut as u8) | ((config.paramp as u8) << 6);

            self.bus
                .write_reg_u8(Self::abs_reg(regs::RG_RFXX_TXCUTC), txcutc)?;
        }

        // Transmitter Power Amplifier Control
        {
            let mut pac = 0u8;

            pac = pac | core::cmp::min(31, config.power);
            pac = pac | ((config.pacur as u8) << 5);

            self.bus
                .write_reg_u8(Self::abs_reg(regs::RG_RFXX_PAC), pac)?;
        }

        Ok(())
    }

    pub fn configure_receiver(&mut self, config: &RadioReceiverConfig) -> Result<(), RadioError> {
        // Receiver Digital Frontend
        {
            let mut rxdfe = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_RXDFE))?;

            // Clear SR and RCUT bits
            rxdfe = rxdfe & 0b0001_0000;
            rxdfe = rxdfe | (config.sr as u8) | ((config.rcut as u8) << 5);

            self.bus
                .write_reg_u8(Self::abs_reg(regs::RG_RFXX_RXDFE), rxdfe)?;
        }

        // Receiver Filter Bandwidth Control
        {
            let mut rxbwc = self.bus.read_reg_u8(Self::abs_reg(regs::RG_RFXX_RXBWC))?;

            rxbwc = rxbwc & 0b1100_0000;
            rxbwc = rxbwc | (config.bw as u8);

            if config.if_inversion {
                rxbwc = rxbwc | 0b0010_0000;
            }

            if config.if_shift {
                rxbwc = rxbwc | 0b0001_0000;
            }

            self.bus
                .write_reg_u8(Self::abs_reg(regs::RG_RFXX_RXBWC), rxbwc)?;
        }

        Ok(())
    }

    pub fn configure_transreceiver(
        &mut self,
        config: &RadioTransreceiverConfig,
    ) -> Result<(), RadioError> {
        self.configure_transmitter(&config.tx_config)?;
        self.configure_receiver(&config.rx_config)?;
        self.set_agc_control(&config.agc_control)?;
        self.set_agc_gain(&config.agc_gain)?;
        self.set_ed_duration(config.edd)?;

        Ok(())
    }

    pub fn set_control_pad(&mut self, config: FrontendPinConfig) -> Result<(), RadioError> {
        let padfe = (config as u8) << 6;

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_PADFE), padfe)?;

        Ok(())
    }

    pub fn set_agc_control(&mut self, agc_control: &AgcReceiverControl) -> Result<(), RadioError> {
        let mut agcc = 0u8;

        if agc_control.enabled {
            agcc = agcc | 0b0000_0001;
        }

        if agc_control.agc_input {
            agcc = agcc | 0b0100_0000;
        }

        if agc_control.freeze_control {
            agcc = agcc | 0b0000_0010;
        }

        if agc_control.reset {
            agcc = agcc | 0b0000_1000;
        }

        agcc = agcc | ((agc_control.average_time as u8) << 4);

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_AGCC), agcc)?;

        Ok(())
    }

    pub fn set_agc_gain(&mut self, agc_gain: &AgcReceiverGain) -> Result<(), RadioError> {
        let mut agcs = 0u8;

        agcs = agcs | ((agc_gain.target_level as u8) << 5);
        agcs = agcs | core::cmp::min(23, agc_gain.gcw);

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_AGCS), agcs)?;

        Ok(())
    }

    pub fn set_aux_settings(&mut self, settings: AuxiliarySettings) -> Result<(), RadioError> {
        let mut auxs = 0u8;

        auxs = auxs | (settings.map as u8) << 5;
        auxs = auxs | (settings.pavol as u8);

        if settings.ext_lna_bypass {
            auxs = auxs | 0b1000_0000;
        }

        if settings.aven {
            auxs = auxs | 0b0000_1000;
        }

        if settings.avect {
            auxs = auxs | 0b0001_0000;
        }

        self.bus
            .write_reg_u8(Self::abs_reg(regs::RG_RFXX_AUXS), auxs)?;

        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), RadioError> {
        self.bus.hardware_reset().map_err(RadioError::from)?;

        self.set_state(RadioState::TrxOff)?;

        Ok(())
    }

    pub const fn check_band(freq: RadioFrequency) -> bool {
        (freq <= B::MAX_FREQUENCY) && (freq >= B::MIN_FREQUENCY)
    }

    /// Returns absolute register address for a specified `Band`
    const fn abs_reg(addr: RegisterAddress) -> RegisterAddress {
        B::RADIO_ADDRESS + addr
    }
}

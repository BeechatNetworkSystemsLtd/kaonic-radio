#![allow(dead_code)]

use core::marker::PhantomData;

/// AT86RF215 Datasheet: Register Summary

pub type RegisterAddress = u16;
pub type RegisterValue = u8;

pub(crate) const RG_RFXX_FREQ_RESOLUTION_HZ: u32 = 25000;

// Operation Modificators
pub(crate) const RG_OP_WRITE: RegisterAddress = 0x8000;
pub(crate) const RG_OP_READ: RegisterAddress = 0x0000;

// Address space offset's
pub(crate) const RG_RF09_BASE_ADDRESS: RegisterAddress = 0x0100;
pub(crate) const RG_RF24_BASE_ADDRESS: RegisterAddress = 0x0200;
pub(crate) const RG_BBC0_BASE_ADDRESS: RegisterAddress = 0x0300;
pub(crate) const RG_BBC1_BASE_ADDRESS: RegisterAddress = 0x0400;
pub(crate) const RG_BBC0_FRAME_BUFFER_ADDRESS: RegisterAddress = 0x2000;
pub(crate) const RG_BBC1_FRAME_BUFFER_ADDRESS: RegisterAddress = 0x3000;

// IRQ Status Registers
pub(crate) const RG_RF09_IRQS: RegisterAddress = 0x00;
pub(crate) const RG_RF24_IRQS: RegisterAddress = 0x01;
pub(crate) const RG_BBC0_IRQS: RegisterAddress = 0x02;
pub(crate) const RG_BBC1_IRQS: RegisterAddress = 0x03;

// Common Registers
pub(crate) const RG_RF_RST: RegisterAddress = 0x05;
pub(crate) const RG_RF_CFG: RegisterAddress = 0x06;
pub(crate) const RG_RF_CLKO: RegisterAddress = 0x07;
pub(crate) const RG_RF_BMDVC: RegisterAddress = 0x08;
pub(crate) const RG_RF_XOC: RegisterAddress = 0x09;
pub(crate) const RG_RF_IQIFC0: RegisterAddress = 0x0A;
pub(crate) const RG_RF_IQIFC1: RegisterAddress = 0x0B;
pub(crate) const RG_RF_IQIFC2: RegisterAddress = 0x0C;
pub(crate) const RG_RF_PN: RegisterAddress = 0x0D;
pub(crate) const RG_RF_VN: RegisterAddress = 0x0E;

// Radio Registers
pub(crate) const RG_RFXX_IRQM: RegisterAddress = 0x000;
pub(crate) const RG_RFXX_AUXS: RegisterAddress = 0x001;
pub(crate) const RG_RFXX_STATE: RegisterAddress = 0x002;
pub(crate) const RG_RFXX_CMD: RegisterAddress = 0x003;
pub(crate) const RG_RFXX_CS: RegisterAddress = 0x004;
pub(crate) const RG_RFXX_CCF0L: RegisterAddress = 0x005;
pub(crate) const RG_RFXX_CCF0H: RegisterAddress = 0x006;
pub(crate) const RG_RFXX_CNL: RegisterAddress = 0x007;
pub(crate) const RG_RFXX_CNM: RegisterAddress = 0x008;
pub(crate) const RG_RFXX_RXBWC: RegisterAddress = 0x009;
pub(crate) const RG_RFXX_RXDFE: RegisterAddress = 0x00A;
pub(crate) const RG_RFXX_AGCC: RegisterAddress = 0x00B;
pub(crate) const RG_RFXX_AGCS: RegisterAddress = 0x00C;
pub(crate) const RG_RFXX_RSSI: RegisterAddress = 0x00D;
pub(crate) const RG_RFXX_EDC: RegisterAddress = 0x00E;
pub(crate) const RG_RFXX_EDD: RegisterAddress = 0x00F;
pub(crate) const RG_RFXX_EDV: RegisterAddress = 0x010;
pub(crate) const RG_RFXX_RNDV: RegisterAddress = 0x011;
pub(crate) const RG_RFXX_TXCUTC: RegisterAddress = 0x012;
pub(crate) const RG_RFXX_TXDFE: RegisterAddress = 0x013;
pub(crate) const RG_RFXX_PAC: RegisterAddress = 0x014;
pub(crate) const RG_RFXX_PADFE: RegisterAddress = 0x016;
pub(crate) const RG_RFXX_PLL: RegisterAddress = 0x021;
pub(crate) const RG_RFXX_PLLCF: RegisterAddress = 0x022;
pub(crate) const RG_RFXX_TXCI: RegisterAddress = 0x025;
pub(crate) const RG_RFXX_TXCQ: RegisterAddress = 0x026;
pub(crate) const RG_RFXX_TXDACI: RegisterAddress = 0x027;
pub(crate) const RG_RFXX_TXDACQ: RegisterAddress = 0x028;

// Baseband Registers
pub(crate) const RG_BBCX_IRQM: RegisterAddress = 0x000;
pub(crate) const RG_BBCX_PC: RegisterAddress = 0x001;
pub(crate) const RG_BBCX_PS: RegisterAddress = 0x002;
pub(crate) const RG_BBCX_RXFLL: RegisterAddress = 0x004;
pub(crate) const RG_BBCX_RXFLH: RegisterAddress = 0x005;
pub(crate) const RG_BBCX_TXFLL: RegisterAddress = 0x006;
pub(crate) const RG_BBCX_TXFLH: RegisterAddress = 0x007;
pub(crate) const RG_BBCX_FBLL: RegisterAddress = 0x008;
pub(crate) const RG_BBCX_FBLH: RegisterAddress = 0x009;
pub(crate) const RG_BBCX_FBLIL: RegisterAddress = 0x00A;
pub(crate) const RG_BBCX_FBLIH: RegisterAddress = 0x00B;
pub(crate) const RG_BBCX_OFDMPHRTX: RegisterAddress = 0x00C;
pub(crate) const RG_BBCX_OFDMPHRRX: RegisterAddress = 0x00D;
pub(crate) const RG_BBCX_OFDMC: RegisterAddress = 0x00E;
pub(crate) const RG_BBCX_OFDMSW: RegisterAddress = 0x00F;
pub(crate) const RG_BBCX_OQPSKC0: RegisterAddress = 0x010;
pub(crate) const RG_BBCX_OQPSKC1: RegisterAddress = 0x011;
pub(crate) const RG_BBCX_OQPSKC2: RegisterAddress = 0x012;
pub(crate) const RG_BBCX_OQPSKC3: RegisterAddress = 0x013;
pub(crate) const RG_BBCX_OQPSKPHRTX: RegisterAddress = 0x014;
pub(crate) const RG_BBCX_OQPSKPHRRX: RegisterAddress = 0x015;
pub(crate) const RG_BBCX_AFC0: RegisterAddress = 0x020;
pub(crate) const RG_BBCX_AFC1: RegisterAddress = 0x021;
pub(crate) const RG_BBCX_AFFTM: RegisterAddress = 0x022;
pub(crate) const RG_BBCX_AFFVM: RegisterAddress = 0x023;
pub(crate) const RG_BBCX_AFS: RegisterAddress = 0x024;
pub(crate) const RG_BBCX_MACEA0: RegisterAddress = 0x025;
pub(crate) const RG_BBCX_MACEA1: RegisterAddress = 0x026;
pub(crate) const RG_BBCX_MACEA2: RegisterAddress = 0x027;
pub(crate) const RG_BBCX_MACEA3: RegisterAddress = 0x028;
pub(crate) const RG_BBCX_MACEA4: RegisterAddress = 0x029;
pub(crate) const RG_BBCX_MACEA5: RegisterAddress = 0x02A;
pub(crate) const RG_BBCX_MACEA6: RegisterAddress = 0x02B;
pub(crate) const RG_BBCX_MACEA7: RegisterAddress = 0x02C;
pub(crate) const RG_BBCX_MACPID0F0: RegisterAddress = 0x02D;
pub(crate) const RG_BBCX_MACPID1F0: RegisterAddress = 0x02E;
pub(crate) const RG_BBCX_MACSHA0F0: RegisterAddress = 0x02F;
pub(crate) const RG_BBCX_MACSHA1F0: RegisterAddress = 0x030;
pub(crate) const RG_BBCX_MACPID0F1: RegisterAddress = 0x031;
pub(crate) const RG_BBCX_MACPID1F1: RegisterAddress = 0x032;
pub(crate) const RG_BBCX_MACSHA0F1: RegisterAddress = 0x033;
pub(crate) const RG_BBCX_MACSHA1F1: RegisterAddress = 0x034;
pub(crate) const RG_BBCX_MACPID0F2: RegisterAddress = 0x035;
pub(crate) const RG_BBCX_MACPID1F2: RegisterAddress = 0x036;
pub(crate) const RG_BBCX_MACSHA0F2: RegisterAddress = 0x037;
pub(crate) const RG_BBCX_MACSHA1F2: RegisterAddress = 0x038;
pub(crate) const RG_BBCX_MACPID0F3: RegisterAddress = 0x039;
pub(crate) const RG_BBCX_MACPID1F3: RegisterAddress = 0x03A;
pub(crate) const RG_BBCX_MACSHA0F3: RegisterAddress = 0x03B;
pub(crate) const RG_BBCX_MACSHA1F3: RegisterAddress = 0x03C;
pub(crate) const RG_BBCX_AMCS: RegisterAddress = 0x040;
pub(crate) const RG_BBCX_AMEDT: RegisterAddress = 0x041;
pub(crate) const RG_BBCX_AMAACKPD: RegisterAddress = 0x042;
pub(crate) const RG_BBCX_AMAACKTL: RegisterAddress = 0x043;
pub(crate) const RG_BBCX_AMAACKTH: RegisterAddress = 0x044;
pub(crate) const RG_BBCX_FSKC0: RegisterAddress = 0x060;
pub(crate) const RG_BBCX_FSKC1: RegisterAddress = 0x061;
pub(crate) const RG_BBCX_FSKC2: RegisterAddress = 0x062;
pub(crate) const RG_BBCX_FSKC3: RegisterAddress = 0x063;
pub(crate) const RG_BBCX_FSKC4: RegisterAddress = 0x064;
pub(crate) const RG_BBCX_FSKPLL: RegisterAddress = 0x065;
pub(crate) const RG_BBCX_FSKSFD0L: RegisterAddress = 0x066;
pub(crate) const RG_BBCX_FSKSFD0H: RegisterAddress = 0x067;
pub(crate) const RG_BBCX_FSKSFD1L: RegisterAddress = 0x068;
pub(crate) const RG_BBCX_FSKSFD1H: RegisterAddress = 0x069;
pub(crate) const RG_BBCX_FSKPHRTX: RegisterAddress = 0x06A;
pub(crate) const RG_BBCX_FSKPHRRX: RegisterAddress = 0x06B;
pub(crate) const RG_BBCX_FSKRPC: RegisterAddress = 0x06C;
pub(crate) const RG_BBCX_FSKRPCONT: RegisterAddress = 0x06D;
pub(crate) const RG_BBCX_FSKRPCOFFT: RegisterAddress = 0x06E;
pub(crate) const RG_BBCX_FSKRRXFLL: RegisterAddress = 0x070;
pub(crate) const RG_BBCX_FSKRRXFLH: RegisterAddress = 0x071;
pub(crate) const RG_BBCX_FSKDM: RegisterAddress = 0x072;
pub(crate) const RG_BBCX_FSKPE0: RegisterAddress = 0x073;
pub(crate) const RG_BBCX_FSKPE1: RegisterAddress = 0x074;
pub(crate) const RG_BBCX_FSKPE2: RegisterAddress = 0x075;
pub(crate) const RG_BBCX_PMUC: RegisterAddress = 0x080;
pub(crate) const RG_BBCX_PMUVAL: RegisterAddress = 0x081;
pub(crate) const RG_BBCX_PMUQF: RegisterAddress = 0x082;
pub(crate) const RG_BBCX_PMUI: RegisterAddress = 0x083;
pub(crate) const RG_BBCX_PMUQ: RegisterAddress = 0x084;
pub(crate) const RG_BBCX_CNTC: RegisterAddress = 0x090;
pub(crate) const RG_BBCX_CNT0: RegisterAddress = 0x091;
pub(crate) const RG_BBCX_CNT1: RegisterAddress = 0x092;
pub(crate) const RG_BBCX_CNT2: RegisterAddress = 0x093;
pub(crate) const RG_BBCX_CNT3: RegisterAddress = 0x094;

// Baseband Frame Buffer Registers
pub(crate) const RG_BBCX_FBRXS: RegisterAddress = 0x0000;
pub(crate) const RG_BBCX_FBRXE: RegisterAddress = 0x07FE;
pub(crate) const RG_BBCX_FBTXS: RegisterAddress = 0x0800;
pub(crate) const RG_BBCX_FBTXE: RegisterAddress = 0x0FFE;
pub(crate) const RG_BBCX_FRAME_SIZE: usize = 2048;

/// 5.3.2.3 RFn_IRQS – Radio IRQ Status
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum RadioInterrupt {
    /// This bit is set to 1 if the wake-up procedure from state SLEEP/DEEP_SLEEP or power-up procedure is completed. It
    /// also indicates the completion of the RESET procedure.
    Wakeup = 0b0000_0001,

    /// This bit is set to 1 if the command TXPREP is written to the register RFn_CMD and transceiver reaches the state
    /// TXPREP. While being in the state TXPREP and changing the RF frequency, the IRQ TRXRDY is issued once the
    /// frequency settling is completed. Note: It is not set if the baseband switches automatically to the state TXPREP due to
    /// an IRQ TXFE or RXFE.
    TransceiverReady = 0b0000_0010,

    /// This bit is set to 1 if a single or continuous energy measurement is completed. It is not set if the automatic energy
    /// measurement mode is used
    EnergyDetectionCompletion = 0b0000_0100,

    /// This bit is set to 1 if the battery monitor detects a voltage at EVDD that is below the threshold voltage
    BatteryLow = 0b0000_1000,

    /// This bit is set to 1 if a transceiver error is detected, i.e. a PLL lock error occurs
    TransceiverError = 0b0001_0000,

    /// This bit is set to 1 if the I/Q data interface synchronization fails.
    IqIfSyncFail = 0b0010_0000,
}

impl Into<u8> for RadioInterrupt {
    fn into(self) -> u8 {
        self as u8
    }
}

/// 5.3.2.4 BBCn_IRQS – Baseband IRQ Status
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum BasebandInterrupt {
    ReceiverFrameStart = 0b0000_0001,
    ReceiverFrameEnd = 0b0000_0010,
    ReceiverAddressMatch = 0b0000_0100,
    ReceiverExtendedMatch = 0b0000_1000,
    TransmitterFrameEnd = 0b0001_0000,
    AgcHold = 0b0010_0000,
    AgcRelease = 0b0100_0000,
    FrameBufferLevelIndication = 0b1000_0000,
}

impl Into<u8> for BasebandInterrupt {
    fn into(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InterruptMask<I: Into<u8>> {
    mask: u8,
    _irq: PhantomData<I>,
}

impl<I: Into<u8>> InterruptMask<I> {
    pub fn new() -> Self {
        Self {
            mask: 0,
            _irq: PhantomData::default(),
        }
    }

    pub fn new_from_mask(mask: u8) -> Self {
        Self {
            mask: mask,
            _irq: PhantomData::default(),
        }
    }

    pub fn fill(&mut self) -> &mut Self {
        self.mask = 0b1111_1111;
        self
    }

    pub fn add_irq(&mut self, irq: I) -> &mut Self {
        self.mask = self.mask | irq.into();
        self
    }

    pub fn has_irq(&self, irq: I) -> bool {
        (self.mask & irq.into()) != 0
    }

    pub fn has_irqs(&self, irqs: InterruptMask<I>) -> bool {
        (self.mask & irqs.mask) == irqs.mask
    }

    pub fn clear_irq(&mut self, irq: I) -> &mut Self {
        self.mask = self.mask & (!(irq.into()));
        self
    }

    pub fn reset(&mut self) -> &mut Self {
        self.mask = 0;
        self
    }

    pub fn build(self) -> Self {
        self
    }

    pub fn get(&self) -> u8 {
        self.mask
    }
}

pub type RadioInterruptMask = InterruptMask<RadioInterrupt>;
pub type BasebandInterruptMask = InterruptMask<BasebandInterrupt>;

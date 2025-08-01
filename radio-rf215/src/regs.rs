/// AT86RF215 Datasheet: Register Summary

pub type RegisterAddress = u16;
pub type RegisterValue = u8;

pub(crate) const RG_RFXX_FREQ_RESOLUTION_KHZ: u32 = 25;

pub(crate) const RG_OP_WRITE: RegisterAddress = 0x8000;
pub(crate) const RG_OP_READ: RegisterAddress = 0x0000;

pub(crate) const RG_RF09_BASE_ADDRESS: RegisterAddress = 0x0100;
pub(crate) const RG_RF24_BASE_ADDRESS: RegisterAddress = 0x0200;

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

/// 5.3.2.3 RFn_IRQS – Radio IRQ Status
pub(crate) enum RadioInterruptStatus {
    Wakeup = 0b0000_0001,

    /// This bit is set to 1 if the command TXPREP is written to the register RFn_CMD and transceiver reaches the state
    /// TXPREP. While being in the state TXPREP and changing the RF frequency, the IRQ TRXRDY is issued once the
    /// frequency settling is completed. Note: It is not set if the baseband switches automatically to the state TXPREP due to
    /// an IRQ TXFE or RXFE.
    TransceiverReady = 0b0000_0010,

    EnergyDetectionCompletion = 0b0000_0100,

    BatteryLow,
}

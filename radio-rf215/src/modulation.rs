pub enum Modulation {
    Off,
    Ofdm(OfdmModulation),
    Qpsk,
    Fsk,
}

///  Modulation and Coding Scheme
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum OfdmMcs {
    BpskC1_2_4x = 0x00, // BPSK, coding rate 1/2, 4 x frequency repetition
    BpskC1_2_2x = 0x01, // BPSK, coding rate 1/2, 2 x frequency repetition
    QpskC1_2_2x = 0x02, // QPSK, coding rate 1/2, 2 x frequency repetition
    QpskC1_2 = 0x03,    // QPSK, coding rate 1/2
    QpskC3_4 = 0x04,    // QPSK, coding rate 3/4
    QamC1_2 = 0x05,     // 16-QAM, coding rate 1/2
    QamC3_4 = 0x06,     // 16-QAM, coding rate 3/4
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum OfdmBandwidthOption {
    Option1 = 0x00,
    Option2 = 0x01,
    Option3 = 0x02,
    Option4 = 0x03,
}

pub struct OfdmModulation {
    pub mcs: OfdmMcs,
    pub opt: OfdmBandwidthOption,
    pub pdt: u8, // Preamble Detection Threshold
}

impl Default for OfdmModulation {
    fn default() -> Self {
        Self {
            mcs: OfdmMcs::BpskC1_2_4x,
            opt: OfdmBandwidthOption::Option1,
            pdt: 0x03,
        }
    }
}

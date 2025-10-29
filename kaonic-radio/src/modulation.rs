use core::fmt;

pub enum Modulation {
    Ofdm(OfdmModulation),
    Qpsk(QpskModulation),
}

impl fmt::Display for Modulation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Modulation::Ofdm(ofdm) => {
                writeln!(
                    f,
                    "[OFDM mcs:{} opt:{} pwr:{}]",
                    ofdm.mcs as u8, ofdm.opt as u8, ofdm.tx_power
                )?;
            }
            Modulation::Qpsk(qpsk) => {
                writeln!(
                    f,
                    "[QPSK freq:{} mode:{} pwr:{}]",
                    qpsk.chip_freq as u8, qpsk.mode as u8, qpsk.tx_power
                )?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum OfdmMcs {
    Mcs0 = 0x00,
    Mcs1 = 0x01,
    Mcs2 = 0x02,
    Mcs3 = 0x03,
    Mcs4 = 0x04,
    Mcs5 = 0x05,
    Mcs6 = 0x06,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum OfdmOption {
    Option1 = 0x00,
    Option2 = 0x01,
    Option3 = 0x02,
    Option4 = 0x03,
}

pub struct OfdmModulation {
    pub mcs: OfdmMcs,
    pub opt: OfdmOption,
    pub tx_power: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum QpskChipFrequency {
    Freq100 = 0x00,
    Freq200 = 0x01,
    Freq1000 = 0x02,
    Freq2000 = 0x03,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum QpskRateMode {
    Mode0 = 0x00,
    Mode1 = 0x01,
    Mode2 = 0x02,
    Mode3 = 0x03,
}

pub struct QpskModulation {
    pub chip_freq: QpskChipFrequency,
    pub mode: QpskRateMode,
    pub tx_power: u8,
}

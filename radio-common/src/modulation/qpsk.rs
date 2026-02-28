use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum QpskChipFrequency {
    Fchip100 = 0x00,
    Fchip200 = 0x01,
    Fchip1000 = 0x02,
    Fchip2000 = 0x03,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum QpskRateMode {
    RateMode0 = 0x00,
    RateMode1 = 0x01,
    RateMode2 = 0x02,
    RateMode3 = 0x03,
    RateMode4 = 0x04,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QpskModulation {
    pub fchip: QpskChipFrequency,
    pub mode: QpskRateMode,
    pub tx_power: u8,
}

impl Default for QpskModulation {
    fn default() -> Self {
        Self {
            fchip: QpskChipFrequency::Fchip100,
            mode: QpskRateMode::RateMode0,
            tx_power: 10,
        }
    }
}

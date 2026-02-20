mod ofdm;
mod qpsk;

use core::fmt;

use serde::{Deserialize, Serialize};

pub use ofdm::*;
pub use qpsk::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Modulation {
    Off,
    Ofdm(OfdmModulation),
    Qpsk(QpskModulation),
    Fsk,
}

impl Modulation {
    pub fn tx_power(&self) -> u8 {
        match self {
            Modulation::Off => 0,
            Modulation::Ofdm(ofdm) => ofdm.tx_power,
            Modulation::Qpsk(qpsk) => qpsk.tx_power,
            Modulation::Fsk => 0,
        }
    }
}

impl fmt::Display for Modulation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[mod] ({} dBm) -> ", self.tx_power())?;

        match self {
            Modulation::Ofdm(ofdm) => {
                write!(f, "OFDM (mcs:{} opt:{})", ofdm.mcs as u8, ofdm.opt as u8)?;
            }
            Modulation::Qpsk(qpsk) => {
                write!(
                    f,
                    "QPSK (freq:{} mode:{}]",
                    qpsk.fchip as u8, qpsk.mode as u8,
                )?;
            }
            Modulation::Off => {
                write!(f, "OFF")?;
            }
            Modulation::Fsk => {
                write!(f, "FSK (...")?;
            }
        }

        Ok(())
    }
}

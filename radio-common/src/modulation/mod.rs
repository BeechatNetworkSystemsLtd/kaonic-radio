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
    /// PHY payload rate, for computing airtime. Reports the lower bound of
    /// each setting so a derived timeout is never shorter than the
    /// transmission, and never zero so it is safe as a divisor.
    pub fn data_rate_bps(&self) -> u32 {
        match self {
            Modulation::Ofdm(ofdm) => {
                // Option 1 rates; each narrower option halves them.
                let base = match ofdm.mcs {
                    OfdmMcs::BpskC1_2_4x => 100_000,
                    OfdmMcs::BpskC1_2_2x => 200_000,
                    OfdmMcs::QpskC1_2_2x => 400_000,
                    OfdmMcs::QpskC1_2 => 800_000,
                    OfdmMcs::QpskC3_4 => 1_200_000,
                    OfdmMcs::QamC1_2 => 1_600_000,
                    OfdmMcs::QamC3_4 => 2_400_000,
                };
                let divider = match ofdm.opt {
                    OfdmBandwidthOption::Option1 => 1,
                    OfdmBandwidthOption::Option2 => 2,
                    OfdmBandwidthOption::Option3 => 4,
                    OfdmBandwidthOption::Option4 => 8,
                };
                base / divider
            }
            // Chip rate divided by the spreading the rate mode selects.
            Modulation::Qpsk(qpsk) => {
                // Rate mode 4 exists only at 2000 kchip/s; elsewhere the chip
                // falls back to rate mode 0.
                let rates: [u32; 5] = match qpsk.fchip {
                    QpskChipFrequency::Fchip100 => [6_250, 12_500, 25_000, 50_000, 6_250],
                    QpskChipFrequency::Fchip200 => [12_500, 25_000, 50_000, 100_000, 12_500],
                    QpskChipFrequency::Fchip1000 => [31_250, 125_000, 250_000, 500_000, 31_250],
                    QpskChipFrequency::Fchip2000 => [31_250, 125_000, 250_000, 500_000, 1_000_000],
                };
                rates[qpsk.mode as usize % rates.len()]
            }
            // Common MR-FSK operating mode here.
            Modulation::Fsk => 50_000,
            Modulation::Off => 6_250,
        }
    }

    /// How long `bytes` of PHY payload take to transmit at this setting.
    pub fn airtime_micros(&self, bytes: usize) -> u64 {
        let bits = (bytes as u64).saturating_mul(8);
        bits.saturating_mul(1_000_000) / u64::from(self.data_rate_bps().max(1))
    }

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

#[cfg(test)]
mod rate_tests {
    use super::*;

    /// A full frame at the slowest MR-O-QPSK mode is seconds of airtime.
    #[test]
    fn slowest_qpsk_frame_takes_seconds() {
        let slow = Modulation::Qpsk(QpskModulation {
            fchip: QpskChipFrequency::Fchip100,
            mode: QpskRateMode::RateMode0,
            tx_power: 14,
        });
        assert_eq!(slow.data_rate_bps(), 6_250);
        let micros = slow.airtime_micros(2048);
        assert!(
            micros > 2_000_000,
            "expected over 2 s for a 2 kB frame, got {micros} us"
        );
    }

    /// The same frame at OFDM is milliseconds.
    #[test]
    fn ofdm_frame_is_milliseconds() {
        let fast = Modulation::Ofdm(OfdmModulation {
            mcs: OfdmMcs::QamC3_4,
            opt: OfdmBandwidthOption::Option1,
            tx_power: 14,
        });
        assert_eq!(fast.data_rate_bps(), 2_400_000);
        let micros = fast.airtime_micros(2048);
        assert!(micros < 10_000, "expected under 10 ms, got {micros} us");
    }

    /// Narrower bandwidth options are proportionally slower.
    #[test]
    fn narrow_ofdm_options_are_slower() {
        let opt = |o| {
            Modulation::Ofdm(OfdmModulation {
                mcs: OfdmMcs::QamC3_4,
                opt: o,
                tx_power: 14,
            })
            .data_rate_bps()
        };
        assert_eq!(opt(OfdmBandwidthOption::Option1), 2_400_000);
        assert_eq!(opt(OfdmBandwidthOption::Option4), 300_000);
    }

    /// Never zero; the rate is used as a divisor.
    #[test]
    fn every_setting_reports_a_usable_rate() {
        for m in [Modulation::Off, Modulation::Fsk] {
            assert!(m.data_rate_bps() > 0);
        }
    }
}

use crate::{error::KaonicError, modulation::Modulation};

pub(crate) fn map_modulation(
    modulation: &Modulation,
) -> Result<radio_rf215::modulation::Modulation, KaonicError> {
    match modulation {
        Modulation::Ofdm(ofdm) => {
            let mut result = radio_rf215::modulation::OfdmModulation::default();

            result.tx_power = ofdm.tx_power;

            result.mcs = match ofdm.mcs {
                crate::modulation::OfdmMcs::Mcs0 => radio_rf215::modulation::OfdmMcs::BpskC1_2_4x,
                crate::modulation::OfdmMcs::Mcs1 => radio_rf215::modulation::OfdmMcs::BpskC1_2_2x,
                crate::modulation::OfdmMcs::Mcs2 => radio_rf215::modulation::OfdmMcs::QpskC1_2_2x,
                crate::modulation::OfdmMcs::Mcs3 => radio_rf215::modulation::OfdmMcs::QpskC1_2,
                crate::modulation::OfdmMcs::Mcs4 => radio_rf215::modulation::OfdmMcs::QpskC3_4,
                crate::modulation::OfdmMcs::Mcs5 => radio_rf215::modulation::OfdmMcs::QamC1_2,
                crate::modulation::OfdmMcs::Mcs6 => radio_rf215::modulation::OfdmMcs::QamC3_4,
            };

            result.opt = match ofdm.opt {
                crate::modulation::OfdmOption::Option1 => {
                    radio_rf215::modulation::OfdmBandwidthOption::Option1
                }
                crate::modulation::OfdmOption::Option2 => {
                    radio_rf215::modulation::OfdmBandwidthOption::Option2
                }
                crate::modulation::OfdmOption::Option3 => {
                    radio_rf215::modulation::OfdmBandwidthOption::Option3
                }
                crate::modulation::OfdmOption::Option4 => {
                    radio_rf215::modulation::OfdmBandwidthOption::Option4
                }
            };

            return Ok(radio_rf215::modulation::Modulation::Ofdm(result));
        }
        Modulation::Qpsk(qpsk) => {
            let mut result = radio_rf215::modulation::QpskModulation::default();

            result.tx_power = qpsk.tx_power;

            result.fchip = match qpsk.chip_freq {
                crate::modulation::QpskChipFrequency::Freq100 => {
                    radio_rf215::modulation::QpskChipFrequency::Fchip100
                }
                crate::modulation::QpskChipFrequency::Freq200 => {
                    radio_rf215::modulation::QpskChipFrequency::Fchip200
                }
                crate::modulation::QpskChipFrequency::Freq1000 => {
                    radio_rf215::modulation::QpskChipFrequency::Fchip1000
                }
                crate::modulation::QpskChipFrequency::Freq2000 => {
                    radio_rf215::modulation::QpskChipFrequency::Fchip2000
                }
            };

            result.mode = match qpsk.mode {
                crate::modulation::QpskRateMode::Mode0 => {
                    radio_rf215::modulation::QpskRateMode::RateMode0
                }
                crate::modulation::QpskRateMode::Mode1 => {
                    radio_rf215::modulation::QpskRateMode::RateMode1
                }
                crate::modulation::QpskRateMode::Mode2 => {
                    radio_rf215::modulation::QpskRateMode::RateMode2
                }
                crate::modulation::QpskRateMode::Mode3 => {
                    radio_rf215::modulation::QpskRateMode::RateMode3
                }
            };

            return Ok(radio_rf215::modulation::Modulation::Qpsk(result));
        }
    }
}

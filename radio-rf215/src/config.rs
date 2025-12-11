use crate::{
    bus::Bus,
    modulation::{self, Modulation},
    radio::{
        AgcAverageTime, AgcTargetLevel, FrequencySampleRate, PaCur, PaRampTime,
        RadioTransreceiverConfig, ReceiverBandwidth, RelativeCutOff, TransmitterCutOff,
    },
    transceiver::{Band09, Band24, Transreceiver},
};

pub trait TransreceiverConfigurator {
    fn create_modulation_config(&self, modulation: &Modulation) -> RadioTransreceiverConfig;
}

// Recommended configuration for sub-GHz band
impl<I: Bus + Clone> TransreceiverConfigurator for Transreceiver<Band09, I> {
    fn create_modulation_config(&self, modulation: &Modulation) -> RadioTransreceiverConfig {
        let mut trx_config = RadioTransreceiverConfig::default();
        let tx_config = &mut trx_config.tx_config;
        let rx_config = &mut trx_config.rx_config;
        let agc_control = &mut trx_config.agc_control;
        let agc_gain = &mut trx_config.agc_gain;

        match modulation {
            Modulation::Ofdm(ofdm) => {
                // Table 6-90. Recommended Transmitter Frontend Configuration
                // Table 6-93. Recommended PHY Receiver and Digital Frontend Configuration

                trx_config.edd = core::time::Duration::from_micros(960);
                agc_control.average_time = crate::radio::AgcAverageTime::Samples8;
                agc_control.agc_input = false;

                match ofdm.opt {
                    modulation::OfdmBandwidthOption::Option1 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut1_000;
                        tx_config.lpfcut = TransmitterCutOff::Flc800kHz;

                        rx_config.rcut = RelativeCutOff::Fcut1_000;
                        rx_config.bw = ReceiverBandwidth::Bw1250kHzIf2000kHz;
                        rx_config.if_shift = true;
                    }
                    modulation::OfdmBandwidthOption::Option2 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc500kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw800kHzIf1000kHz;
                        rx_config.if_shift = true;
                    }
                    modulation::OfdmBandwidthOption::Option3 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc250kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw400kHzIf500kHz;
                        rx_config.if_shift = false;
                    }
                    modulation::OfdmBandwidthOption::Option4 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_500;
                        tx_config.lpfcut = TransmitterCutOff::Flc160kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw250kHzIf250kHz;
                        rx_config.if_shift = true;
                    }
                };

                rx_config.sr = tx_config.sr;
                tx_config.power = ofdm.tx_power;
            }
            Modulation::Qpsk(qpsk) => {
                // Table 6-106. O-QPSK Receiver Frontend Configuration (AGC Settings)
                agc_control.enabled = true;
                agc_gain.target_level = AgcTargetLevel::TargetN30dB;

                match qpsk.fchip {
                    modulation::QpskChipFrequency::Fchip100 => {
                        agc_control.average_time = AgcAverageTime::Samples32;

                        tx_config.sr = FrequencySampleRate::SampleRate400kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc400kHz;
                        tx_config.paramp = PaRampTime::Paramp32;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw160kHzIf250kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate400kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(10 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip200 => {
                        agc_control.average_time = AgcAverageTime::Samples32;

                        tx_config.paramp = PaRampTime::Paramp16;
                        tx_config.sr = FrequencySampleRate::SampleRate800kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc400kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw250kHzIf250kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate800kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(5 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip1000 => {
                        agc_control.average_time = AgcAverageTime::Samples8;

                        tx_config.paramp = PaRampTime::Paramp4;
                        tx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc1000kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_250;
                        rx_config.bw = ReceiverBandwidth::Bw1000kHzIf1000kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(4 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip2000 => {
                        agc_control.average_time = AgcAverageTime::Samples8;

                        tx_config.paramp = PaRampTime::Paramp4;
                        tx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        tx_config.rcut = RelativeCutOff::Fcut1_000;
                        tx_config.lpfcut = TransmitterCutOff::Flc1000kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw2000kHzIf2000kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(4 * 128);
                    }
                }

                tx_config.power = qpsk.tx_power;
            }
            _ => {}
        }

        trx_config.tx_config.pacur = PaCur::NoReduction;

        return trx_config;
    }
}

// Recommended configuration for 2.4GHz band
impl<I: Bus + Clone> TransreceiverConfigurator for Transreceiver<Band24, I> {
    fn create_modulation_config(&self, modulation: &Modulation) -> RadioTransreceiverConfig {
        let mut trx_config = RadioTransreceiverConfig::default();
        let tx_config = &mut trx_config.tx_config;
        let rx_config = &mut trx_config.rx_config;
        let agc_control = &mut trx_config.agc_control;
        let agc_gain = &mut trx_config.agc_gain;

        match modulation {
            Modulation::Ofdm(ofdm) => {
                // Table 6-90. Recommended Transmitter Frontend Configuration
                // Table 6-93. Recommended PHY Receiver and Digital Frontend Configuration

                trx_config.edd = core::time::Duration::from_micros(960);
                agc_control.average_time = crate::radio::AgcAverageTime::Samples8;
                agc_control.agc_input = false;

                // TODO: Configure OFDM.LFO (Reception with Low Frequency Offset)
                let ofdm_lfo = false;

                match ofdm.opt {
                    modulation::OfdmBandwidthOption::Option1 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut1_000;
                        tx_config.lpfcut = TransmitterCutOff::Flc800kHz;

                        if !ofdm_lfo {
                            rx_config.rcut = RelativeCutOff::Fcut1_000;
                            rx_config.bw = ReceiverBandwidth::Bw1600kHzIf2000kHz;
                            rx_config.if_shift = true;
                        } else {
                            rx_config.rcut = RelativeCutOff::Fcut1_000;
                            rx_config.bw = ReceiverBandwidth::Bw1250kHzIf2000kHz;
                            rx_config.if_shift = true;
                        }
                    }
                    modulation::OfdmBandwidthOption::Option2 => {
                        tx_config.sr = FrequencySampleRate::SampleRate1333kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc500kHz;

                        if !ofdm_lfo {
                            rx_config.rcut = RelativeCutOff::Fcut0_500;
                            rx_config.bw = ReceiverBandwidth::Bw800kHzIf1000kHz;
                            rx_config.if_shift = true;
                        } else {
                            rx_config.rcut = RelativeCutOff::Fcut0_500;
                            rx_config.bw = ReceiverBandwidth::Bw800kHzIf1000kHz;
                            rx_config.if_shift = true;
                        }
                    }
                    modulation::OfdmBandwidthOption::Option3 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc250kHz;

                        if !ofdm_lfo {
                            rx_config.rcut = RelativeCutOff::Fcut0_750;
                            rx_config.bw = ReceiverBandwidth::Bw500kHzIf500kHz;
                            rx_config.if_shift = true;
                        } else {
                            rx_config.rcut = RelativeCutOff::Fcut0_500;
                            rx_config.bw = ReceiverBandwidth::Bw400kHzIf500kHz;
                            rx_config.if_shift = false;
                        }
                    }
                    modulation::OfdmBandwidthOption::Option4 => {
                        tx_config.sr = FrequencySampleRate::SampleRate666kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_500;
                        tx_config.lpfcut = TransmitterCutOff::Flc160kHz;

                        if !ofdm_lfo {
                            rx_config.rcut = RelativeCutOff::Fcut0_375;
                            rx_config.bw = ReceiverBandwidth::Bw320kHzIf500kHz;
                            rx_config.if_shift = false;
                        } else {
                            rx_config.rcut = RelativeCutOff::Fcut0_375;
                            rx_config.bw = ReceiverBandwidth::Bw250kHzIf250kHz;
                            rx_config.if_shift = true;
                        }
                    }
                };

                rx_config.sr = tx_config.sr;
                tx_config.power = ofdm.tx_power;
            }
            Modulation::Qpsk(qpsk) => {
                // Table 6-106. O-QPSK Receiver Frontend Configuration (AGC Settings)
                agc_control.enabled = true;
                agc_gain.target_level = AgcTargetLevel::TargetN30dB;

                match qpsk.fchip {
                    modulation::QpskChipFrequency::Fchip100 => {
                        agc_control.average_time = AgcAverageTime::Samples32;

                        tx_config.sr = FrequencySampleRate::SampleRate400kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc400kHz;
                        tx_config.paramp = PaRampTime::Paramp32;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw160kHzIf250kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate400kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(10 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip200 => {
                        agc_control.average_time = AgcAverageTime::Samples32;

                        tx_config.paramp = PaRampTime::Paramp16;
                        tx_config.sr = FrequencySampleRate::SampleRate800kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc400kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_375;
                        rx_config.bw = ReceiverBandwidth::Bw250kHzIf250kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate800kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(5 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip1000 => {
                        agc_control.average_time = AgcAverageTime::Samples8;

                        tx_config.paramp = PaRampTime::Paramp4;
                        tx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        tx_config.rcut = RelativeCutOff::Fcut0_750;
                        tx_config.lpfcut = TransmitterCutOff::Flc1000kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_250;
                        rx_config.bw = ReceiverBandwidth::Bw1000kHzIf1000kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(4 * 128);
                    }
                    modulation::QpskChipFrequency::Fchip2000 => {
                        agc_control.average_time = AgcAverageTime::Samples8;

                        tx_config.paramp = PaRampTime::Paramp4;
                        tx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        tx_config.rcut = RelativeCutOff::Fcut1_000;
                        tx_config.lpfcut = TransmitterCutOff::Flc1000kHz;

                        rx_config.rcut = RelativeCutOff::Fcut0_500;
                        rx_config.bw = ReceiverBandwidth::Bw2000kHzIf2000kHz;
                        rx_config.sr = FrequencySampleRate::SampleRate4000kHz;
                        rx_config.if_shift = false;

                        trx_config.edd = core::time::Duration::from_micros(4 * 128);
                    }
                }

                tx_config.power = qpsk.tx_power;
            }
            _ => {}
        }

        trx_config.tx_config.pacur = PaCur::NoReduction;

        return trx_config;
    }
}

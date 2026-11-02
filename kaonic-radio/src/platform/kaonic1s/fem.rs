//! Front-end module control: the band filters and the 2.4 GHz antenna switch.
//!
//! These lines hang off the board's I/O expanders and belong to neither radio
//! driver, so both the user-space and the kernel-space radio drive them here.

use radio_common::{frequency::BandwidthFilter, Hertz, RadioConfig};

use crate::{
    error::KaonicError,
    platform::linux::LinuxOutputPin,
    radio::{Antenna, RadioBand},
};

#[derive(Debug)]
pub struct Kaonic1SRadioFem {
    flt_v1: LinuxOutputPin,
    flt_v2: LinuxOutputPin,
    flt_24: Option<LinuxOutputPin>,
    ant_24: Option<LinuxOutputPin>,
    ant_24_sel: Antenna,
}

impl Kaonic1SRadioFem {
    pub fn new(
        flt_v1: LinuxOutputPin,
        flt_v2: LinuxOutputPin,
        flt_24: Option<LinuxOutputPin>,
        ant_24: Option<LinuxOutputPin>,
    ) -> Self {
        Self {
            flt_v1,
            flt_v2,
            flt_24,
            ant_24,
            ant_24_sel: Antenna::default(),
        }
    }

    pub fn set_antenna(&mut self, band: RadioBand, antenna: Antenna) -> Result<(), KaonicError> {
        match band {
            RadioBand::Band24 => {
                if self.ant_24.is_none() && antenna != Antenna::Internal {
                    return Err(KaonicError::NotSupported);
                }

                self.ant_24_sel = antenna;
                self.apply_ant_24()
            }
            // The sub-GHz path has no antenna switch on this board.
            RadioBand::Band09 => match antenna {
                Antenna::Internal => Ok(()),
                Antenna::External => Err(KaonicError::NotSupported),
            },
        }
    }

    pub fn antenna(&self, band: RadioBand) -> Antenna {
        match band {
            RadioBand::Band24 => self.ant_24_sel,
            RadioBand::Band09 => Antenna::Internal,
        }
    }

    fn apply_ant_24(&mut self) -> Result<(), KaonicError> {
        if let Some(ant_24) = &mut self.ant_24 {
            match self.ant_24_sel {
                Antenna::Internal => ant_24.set_low()?,
                Antenna::External => ant_24.set_high()?,
            }
        }

        Ok(())
    }

    fn set_bandwidth_filter(
        &mut self,
        filter: BandwidthFilter,
        freq: Hertz,
    ) -> Result<(), KaonicError> {
        let freq = freq.as_mhz();

        match filter {
            BandwidthFilter::Narrow => {
                log::debug!("set narrowband filter");

                if (902 <= freq) && (freq <= 928) {
                    self.flt_v1.set_high()?;
                    self.flt_v2.set_low()?;
                }

                if (862 <= freq) && (freq <= 876) {
                    self.flt_v1.set_low()?;
                    self.flt_v2.set_high()?;
                }

                // Use Wideband filter
                if freq < 862 {
                    log::trace!(
                        "narrow band is not supported for {}MHz, wideband will be used",
                        freq
                    );

                    self.flt_v1.set_high()?;
                    self.flt_v2.set_low()?;
                }
            }
            BandwidthFilter::Wide => {
                log::debug!("set wideband filter");
                self.flt_v1.set_high()?;
                self.flt_v2.set_low()?;
            }
        }

        Ok(())
    }

    pub fn adjust(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        self.apply_ant_24()?;

        self.set_bandwidth_filter(config.bandwidth_filter, config.freq)?;

        // NOTE: Should be set to 0
        if let Some(flt_24) = &mut self.flt_24 {
            let _ = flt_24.set_low();
        }

        Ok(())
    }
}

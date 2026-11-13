//! Front-end module control: the band filters and the 2.4 GHz antenna switch.
//!
//! These lines hang off the board's I/O expanders and belong to neither radio
//! driver, so both the user-space and the kernel-space radio drive them here.

use radio_common::{frequency::BandwidthFilter, RadioConfig};

use crate::{
    error::KaonicError,
    filter::{self, BandRanges, FilterState},
    platform::{
        kaonic1s::bank::{self, FilterBank, Level},
        linux::LinuxOutputPin,
    },
    radio::{Antenna, RadioBand},
};

#[derive(Debug)]
pub struct Kaonic1SRadioFem {
    /// Which radio this is; scopes the `KAONIC_RF09_FLT_<n>` override.
    index: usize,
    bank: FilterBank,
    ranges: BandRanges,
    /// Indices into `bank.filters`; `None` is auto.
    filter_sel: Option<u8>,
    filter_active: Option<u8>,
    /// The last config's frequency and wide request.
    centre_hz: u64,
    wide: bool,
    flt_v1: LinuxOutputPin,
    flt_v2: LinuxOutputPin,
    flt_24: Option<LinuxOutputPin>,
    ant_24: Option<LinuxOutputPin>,
    ant_24_sel: Antenna,
}

impl Kaonic1SRadioFem {
    pub fn new(
        index: usize,
        bank: FilterBank,
        ranges: BandRanges,
        flt_v1: LinuxOutputPin,
        flt_v2: LinuxOutputPin,
        flt_24: Option<LinuxOutputPin>,
        ant_24: Option<LinuxOutputPin>,
    ) -> Self {
        Self {
            index,
            bank,
            ranges,
            filter_sel: None,
            filter_active: None,
            centre_hz: 0,
            wide: false,
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

    pub fn filter(&self) -> FilterState {
        FilterState {
            filters: self.bank.filters,
            selected: self.filter_sel,
            active: self.filter_active,
        }
    }

    /// Selects a filter (`None` is auto) and applies it.
    pub fn set_filter(
        &mut self,
        index: Option<u8>,
        centre_hz: Option<u64>,
    ) -> Result<(), KaonicError> {
        if index.is_some_and(|index| !filter::exists(self.bank.filters, index)) {
            return Err(KaonicError::IncorrectSettings);
        }

        self.filter_sel = index;

        if let Some(centre_hz) = centre_hz {
            self.centre_hz = centre_hz;
        }

        self.apply_filter()
    }

    pub fn ranges(&self) -> BandRanges {
        self.ranges
    }

    fn apply_filter(&mut self) -> Result<(), KaonicError> {
        let Some(index) =
            filter::resolve(self.bank.filters, self.filter_sel, self.centre_hz, self.wide)
        else {
            return Ok(());
        };
        let filter = &self.bank.filters[usize::from(index)];

        log::debug!(
            "set filter {} ({}) for {} Hz{}",
            index,
            filter.label,
            self.centre_hz,
            if self.filter_sel.is_some() { "" } else { " (auto)" }
        );

        let Some(mut lines) = self.bank.lines(index) else {
            return Ok(());
        };
        self.filter_active = Some(index);

        // Bring-up override, e.g. `KAONIC_RF09_FLT_1=10`.
        if let Ok(text) = std::env::var(format!("KAONIC_RF09_FLT_{}", self.index)) {
            if let Some(forced) = bank::parse_lines(&text) {
                log::info!("radio[{}] rf09 filter override: {}", self.index, text);
                lines = forced;
                self.filter_active = self.bank.by_lines(forced);
            }
        }

        // Raise before lowering: passing through V1=V2=Low shuts the switch down.
        for (pin, level) in [(&mut self.flt_v1, lines.v1), (&mut self.flt_v2, lines.v2)] {
            if level == Level::High {
                pin.set_high()?;
            }
        }
        for (pin, level) in [(&mut self.flt_v1, lines.v1), (&mut self.flt_v2, lines.v2)] {
            if level == Level::Low {
                pin.set_low()?;
            }
        }

        Ok(())
    }

    pub fn adjust(&mut self, config: &RadioConfig) -> Result<(), KaonicError> {
        self.apply_ant_24()?;

        self.centre_hz = centre_hz(config);
        self.wide = config.bandwidth_filter == BandwidthFilter::Wide;
        self.apply_filter()?;

        // NOTE: Should be set to 0
        if let Some(flt_24) = &mut self.flt_24 {
            let _ = flt_24.set_low();
        }

        Ok(())
    }
}

/// The channel's centre frequency.
pub fn centre_hz(config: &RadioConfig) -> u64 {
    config.freq.as_hz() + u64::from(config.channel) * config.channel_spacing.as_hz()
}

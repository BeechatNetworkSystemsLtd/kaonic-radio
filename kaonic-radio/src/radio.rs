use radio_common::{Hertz, Modulation};

use crate::{error::KaonicError, modulation::Modulation};
use core::fmt;

pub type Channel = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandwidthFilter {
    Narrow,
    Wide,
}

#[derive(PartialEq, Clone, Copy)]
pub struct RadioConfig {
    pub freq: Hertz,
    pub channel_spacing: Hertz,
    pub channel: Channel,
    pub bandwidth_filter: BandwidthFilter,
}

pub struct RadioConfigBuilder {
    config: RadioConfig,
}

impl RadioConfigBuilder {
    pub const fn new() -> Self {
        Self {
            config: RadioConfig {
                freq: Hertz::new(869_535_000),
                channel_spacing: Hertz::new(200_000),
                channel: 10,
                bandwidth_filter: BandwidthFilter::Wide,
            },
        }
    }

    pub fn freq(mut self, freq: Hertz) -> Self {
        self.config.freq = freq;
        self
    }

    pub fn channel(mut self, channel: Channel) -> Self {
        self.config.channel = channel;
        self
    }

    pub fn channel_spacing(mut self, spacing: Hertz) -> Self {
        self.config.channel_spacing = spacing;
        self
    }

    pub fn bandwidth_filter(mut self, bandwidth_filter: BandwidthFilter) -> Self {
        self.config.bandwidth_filter = bandwidth_filter;
        self
    }

    pub fn build(self) -> RadioConfig {
        self.config
    }
}

impl fmt::Display for RadioConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "[freq:{} spacing:{} ch:{}]",
            self.freq, self.channel_spacing, self.channel,
        )?;

        Ok(())
    }
}

pub struct ReceiveResult {
    pub rssi: i8,
    pub len: usize,
}

pub struct ScanResult {
    pub rssi: i8,
    pub snr: i8,
}

pub trait Radio {
    type TxFrame;
    type RxFrame;

    fn update_event(&mut self) -> Result<(), KaonicError>;

    fn configure(&mut self, config: &RadioConfig) -> Result<(), KaonicError>;

    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError>;

    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError>;

    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError>;

    fn scan(&mut self, timeout: core::time::Duration) -> Result<ScanResult, KaonicError>;
}

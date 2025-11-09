use crate::{error::KaonicError, modulation::Modulation};
use core::fmt;

pub type Frequency = u32;
pub type Channel = u16;

#[derive(PartialEq, Clone, Copy)]
pub struct RadioConfig {
    pub freq: Frequency,
    pub channel_spacing: Frequency,
    pub channel: Channel,
}

pub struct RadioConfigBuilder {
    config: RadioConfig,
}

impl RadioConfigBuilder {
    pub const fn new() -> Self {
        Self {
            config: RadioConfig {
                freq: 869_535_000,
                channel_spacing: 200_000,
                channel: 10,
            },
        }
    }

    pub fn freq(mut self, freq: Frequency) -> Self {
        self.config.freq = freq;
        self
    }

    pub fn channel(mut self, channel: Channel) -> Self {
        self.config.channel = channel;
        self
    }

    pub fn channel_spacing(mut self, spacing: Frequency) -> Self {
        self.config.channel_spacing = spacing;
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
            "[freq:{}Hz spacing:{}Hz ch:{}]",
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
    pub edv: i8,
}

pub trait Radio {
    type TxFrame;
    type RxFrame;

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

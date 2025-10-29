use crate::{error::KaonicError, modulation::Modulation};
use core::fmt;

pub type Frequency = u32;
pub type Channel = u16;

pub struct RadioConfig {
    pub freq: Frequency,
    pub channel_spacing: Frequency,
    pub channel: Channel,
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
    pub edv: i8,
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

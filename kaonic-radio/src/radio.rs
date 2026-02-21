use radio_common::{Modulation, RadioConfig};

use crate::error::KaonicError;

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

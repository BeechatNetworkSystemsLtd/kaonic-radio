use radio_common::{Modulation, RadioConfig};

use crate::error::KaonicError;

/// Result of a successful frame reception.
pub struct ReceiveResult {
    /// Received signal strength indicator in dBm.
    pub rssi: i8,
    /// Number of bytes in the received frame.
    pub len: usize,
}

/// Result of a channel energy scan.
pub struct ScanResult {
    /// Measured noise floor in dBm.
    pub rssi: i8,
    /// Signal-to-noise ratio in dB.
    pub snr: i8,
}

/// Trait representing a physical radio module.
///
/// Implementors are responsible for managing hardware state including
/// frequency configuration, modulation, frame transmission and reception.
pub trait Radio {
    /// Frame type used for transmission.
    type TxFrame;
    /// Frame type used for reception.
    type RxFrame;

    /// Processes pending hardware interrupt events (e.g. IRQ flags).
    fn update_event(&mut self) -> Result<(), KaonicError>;

    /// Applies a new frequency/channel configuration to the radio hardware.
    fn set_config(&mut self, config: &RadioConfig) -> Result<(), KaonicError>;

    /// Returns the last applied radio configuration.
    fn get_config(&self) -> RadioConfig;

    /// Sets the modulation scheme on the radio hardware.
    fn set_modulation(&mut self, modulation: &Modulation) -> Result<(), KaonicError>;

    /// Returns the current modulation scheme.
    fn get_modulation(&self) -> Modulation;

    /// Transmits a frame over the air.
    fn transmit(&mut self, frame: &Self::TxFrame) -> Result<(), KaonicError>;

    /// Blocks until a frame is received or `timeout` elapses.
    ///
    /// Returns [`KaonicError::Timeout`] if no frame arrives within the timeout.
    fn receive<'a>(
        &mut self,
        frame: &'a mut Self::RxFrame,
        timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError>;

    /// Performs a passive energy scan on the current channel for up to `timeout`.
    fn scan(&mut self, timeout: core::time::Duration) -> Result<ScanResult, KaonicError>;
}

use serde::{Deserialize, Serialize};

/// Frequency band of a radio module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RadioBand {
    /// sub-GHz band (RF09 transceiver)
    Band09,
    /// 2.4 GHz band (RF24 transceiver)
    Band24,
}

/// Antenna selection for a frequency band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Antenna {
    /// On-board (chip/PCB) antenna
    Internal,
    /// External antenna connector
    External,
}

impl Default for Antenna {
    fn default() -> Self {
        Antenna::Internal
    }
}

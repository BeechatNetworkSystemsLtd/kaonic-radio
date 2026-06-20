use serde::{Deserialize, Serialize};

/// Selects how radio baseband processing is performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accelerator {
    /// On-chip baseband processing
    Native,
    /// External FPGA hardware accelerator over the I/Q interface.
    Hardware,
}

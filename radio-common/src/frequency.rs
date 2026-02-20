use core::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Hertz(pub u64);

impl Hertz {
    pub const fn new(hz: u64) -> Self {
        Hertz(hz)
    }

    pub const fn from_khz(khz: u64) -> Self {
        Hertz(khz * 1_000)
    }

    pub const fn from_mhz(mhz: u64) -> Self {
        Hertz(mhz * 1_000_000)
    }

    pub const fn as_hz(&self) -> u64 {
        self.0
    }

    pub const fn as_khz(&self) -> u64 {
        self.0 / 1_000
    }

    pub const fn as_mhz(&self) -> u64 {
        self.0 / 1_000_000
    }
}

impl fmt::Display for Hertz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}Hz", self.0,)?;
        Ok(())
    }
}

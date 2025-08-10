pub mod platform;

use radio_rf215::{Rf215, bus::Bus};

pub enum Machine {
    Kaonic1S,
}

pub type Revision = u32;

pub struct RadioModule<I: Bus> {
    bus: I,
    rf: Rf215<I>,
}

pub struct KaonicRadio {}

impl KaonicRadio {
    pub fn new() -> Self {
        Self {}
    }
}

pub mod platform;

use radio_rf215::{bus::Bus, Rf215};

pub enum Machine {
    Kaonic1S,
}

pub struct RadioModule<I: Bus> {
    pub bus: I,
    pub rf: Rf215<I>,
}

impl<I: Bus> RadioModule<I> {
    pub fn new(bus: I, rf: Rf215<I>) -> Self {
        Self { bus, rf }
    }

}

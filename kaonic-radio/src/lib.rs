pub mod platform;

use radio_rf215::{bus::Bus, Rf215};

pub enum Machine {
    Kaonic1S,
}

pub trait RadioFem {
    fn configure(&mut self, freq: u32);
}

pub struct RadioModule<I: Bus, F: RadioFem> {
    pub bus: I,
    pub rf: Rf215<I>,
    pub fem: F,
}

impl<I: Bus, F: RadioFem> RadioModule<I, F> {
    pub fn new(bus: I, rf: Rf215<I>, fem: F) -> Self {
        Self { bus, rf, fem }
    }
}

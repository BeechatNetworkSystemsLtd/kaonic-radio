pub mod platform;
pub mod error;

pub trait RadioFem {
    fn configure(&mut self, freq: u32);
}

pub struct RadioModule<R, F: RadioFem> {
    pub radio: R,
    pub fem: F,
}

impl<R, F: RadioFem> RadioModule<R, F> {
    pub fn new(radio: R, fem: F) -> Self {
        Self { radio, fem }
    }

    pub fn inner(&mut self) -> &mut R {
        &mut self.radio
    }
}

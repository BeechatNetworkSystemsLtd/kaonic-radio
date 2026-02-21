use crate::{
    error::KaonicError,
    platform::kaonic1s::{Kaonic1SFrame, Kaonic1SMachine, Kaonic1SRadio},
};

pub mod kaonic1s;

pub fn create_machine() -> Result<Kaonic1SMachine, KaonicError> {
    Kaonic1SMachine::new()
}

pub type PlatformRadio = Kaonic1SRadio;
pub type PlatformRadioFrame = Kaonic1SFrame;

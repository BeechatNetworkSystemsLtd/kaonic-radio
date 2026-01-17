use crate::{error::KaonicError, platform::kaonic1s::{Kaonic1SMachine, Kaonic1SRadio}};

pub mod kaonic1s;

mod rf215;

pub fn create_machine() -> Result<Kaonic1SMachine, KaonicError> {
    Kaonic1SMachine::new()
}

pub type PlatformRadio = Kaonic1SRadio;

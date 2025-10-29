use crate::{error::KaonicError, platform::kaonic1s::Kaonic1SMachine};

pub mod kaonic1s;

mod rf215;

pub fn create_machine() -> Result<Kaonic1SMachine, KaonicError> {
    Kaonic1SMachine::new()
}

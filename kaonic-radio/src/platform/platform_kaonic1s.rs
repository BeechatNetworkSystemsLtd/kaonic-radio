use crate::{error::KaonicError, platform::kaonic1s::Kaonic1SMachine};

pub mod kaonic1s;

pub fn create_machine() -> Result<Kaonic1SMachine, KaonicError> {
    Kaonic1SMachine::new()
}

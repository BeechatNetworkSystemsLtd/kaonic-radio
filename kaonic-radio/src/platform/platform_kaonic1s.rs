use crate::{
    error::KaonicError,
    platform::kaonic1s::{Kaonic1SFrame, Kaonic1SMachine, Kaonic1SRadio, Kaonic1SRadioEvent},
};

pub mod kaonic1s;

/// The Kaonic 1X has the 1S's radio section: the same implementation under
/// its own names.
pub mod kaonic1x {
    pub use super::kaonic1s::{
        Kaonic1SFrame as Kaonic1XFrame, Kaonic1SMachine as Kaonic1XMachine,
        Kaonic1SRadio as Kaonic1XRadio, Kaonic1SRadioEvent as Kaonic1XRadioEvent,
        Kaonic1SRadioFem as Kaonic1XRadioFem,
    };
}

pub fn create_machine() -> Result<Kaonic1SMachine, KaonicError> {
    Kaonic1SMachine::new()
}

pub type PlatformRadio = Kaonic1SRadio;
pub type PlatformRadioEvent = Kaonic1SRadioEvent;
pub type PlatformRadioFrame = Kaonic1SFrame;

/// How long a receive should wait for a frame after the radio's event fired.
pub const PLATFORM_RECEIVE_TIMEOUT: core::time::Duration = kaonic1s::RECEIVE_TIMEOUT;

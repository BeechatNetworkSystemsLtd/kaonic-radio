use crate::bus::BusError;

#[derive(Debug, PartialEq, Eq)]
pub enum RadioError {
    IncorrectConfig,
    IncorrectState,
    CommunicationFailure,
}

impl From<BusError> for RadioError {
    fn from(_value: BusError) -> Self {
        Self::CommunicationFailure
    }
}

enum BasebandError {}

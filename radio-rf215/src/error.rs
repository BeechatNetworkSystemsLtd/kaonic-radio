use crate::bus::BusError;

#[derive(Debug, PartialEq, Eq)]
pub enum RadioError {
    IncorrectConfig,
    IncorrectState,
    CommunicationFailure,
    Timeout,
}

impl From<BusError> for RadioError {
    fn from(_value: BusError) -> Self {
        Self::CommunicationFailure
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BasebandError {
    CommunicationFailure,
}

impl From<BusError> for BasebandError {
    fn from(_value: BusError) -> Self {
        Self::CommunicationFailure
    }
}

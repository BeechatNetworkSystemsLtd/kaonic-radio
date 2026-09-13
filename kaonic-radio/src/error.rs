use kaonic_frame::error::FrameError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KaonicError {
    HardwareError,
    IncorrectSettings,
    InvalidState,
    Timeout,
    OutOfMemory,
    PayloadTooBig,
    NotSupported,
    DataCorruption,
    TryAgain,
    /// A transmit was refused because the radio holds a frame that
    /// [`crate::radio::Radio::receive`] has yet to return; receive, then
    /// transmit again.
    RxPending,
}

impl From<FrameError> for KaonicError {
    fn from(_value: FrameError) -> Self {
        KaonicError::OutOfMemory
    }
}

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
}

impl From<FrameError> for KaonicError {
    fn from(_value: FrameError) -> Self {
        KaonicError::OutOfMemory
    }
}

use kaonic_frame::error::FrameError;

#[derive(Debug, Clone, Copy)]
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
    fn from(value: FrameError) -> Self {
        KaonicError::OutOfMemory
    }
}

use kaonic_frame::error::FrameError;

#[derive(Clone, Copy, Debug)]
pub enum NetworkError {
    CorruptedData,
    OutOfMemory,
    PayloadTooBig,
    TryAgain,
    IncorrectSequence,
    NotSupported,
    Busy,
}

impl From<FrameError> for NetworkError {
    fn from(_value: FrameError) -> Self {
        Self::OutOfMemory
    }
}

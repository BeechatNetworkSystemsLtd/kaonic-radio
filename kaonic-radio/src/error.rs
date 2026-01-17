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

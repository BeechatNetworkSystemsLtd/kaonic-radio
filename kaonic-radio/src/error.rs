#[derive(Debug, Clone, Copy)]
pub enum KaonicError {
    HardwareError,
    IncorrectSettings,
    Timeout,
    OutOfMemory,
    NotSupported,
}

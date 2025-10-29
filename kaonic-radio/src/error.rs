#[derive(Debug)]
pub enum KaonicError {
    HardwareError,
    IncorrectSettings,
    Timeout,
    OutOfMemory,
    NotSupported,
}

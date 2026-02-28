#[derive(Clone, Copy, Debug)]
pub enum FrameError {
    OutOfMemory,
    CorruptedData,
    InvalidLength,
}

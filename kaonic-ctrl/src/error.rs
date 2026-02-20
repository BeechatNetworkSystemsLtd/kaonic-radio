use kaonic_frame::error::FrameError;
use kaonic_net::error::NetworkError;
use tokio::io;

#[derive(Clone, Copy, Debug)]
pub enum ControllerError {
    OutOfMemory,
    DecodeError,
    SocketError,
    Timeout,
}

impl From<NetworkError> for ControllerError {
    fn from(_value: NetworkError) -> Self {
        Self::OutOfMemory
    }
}

impl From<io::Error> for ControllerError {
    fn from(_value: io::Error) -> Self {
        Self::SocketError
    }
}

impl From<FrameError> for ControllerError {
    fn from(_value: FrameError) -> Self {
        Self::OutOfMemory
    }
}

use kaonic_net::NetworkTime;

pub mod client;
pub mod error;
pub mod network;
pub mod peer;
pub mod protocol;
pub mod radio;
pub mod server;

/// Returns current system time
pub fn system_time() -> NetworkTime {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");

    duration_since_epoch.as_millis()
}

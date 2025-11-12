pub struct Metrics {
    snr: i32,  // Signal to Noise
    rssi: i32, //
    per: i32,  //
}

pub struct Profile<T> {
    inner: T,
    up_metrics: Metrics,
    down_metrics: Metrics,
}

impl<T> Profile<T> {
}

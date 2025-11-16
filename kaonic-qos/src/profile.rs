/// Performance metrics for radio link quality
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub snr: i32,        // Signal to Noise Ratio in dB
    pub rssi: i32,       // Received Signal Strength Indicator in dBm
    pub per: i32,        // Packet Error Rate (0-100%)
    pub edv: i32,        // Energy Detection Value in dBm
    pub sir: i32,        // Signal to Interference Ratio in dB
    pub sample_count: u32, // Number of samples collected
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            snr: 0,
            rssi: -127,
            per: 0,
            edv: -127,
            sir: 0,
            sample_count: 0,
        }
    }

    /// Update metrics with new EDV reading
    pub fn update_edv(&mut self, edv: i8) {
        if self.sample_count == 0 {
            self.edv = edv as i32;
        } else {
            // Exponential moving average
            self.edv = (self.edv * 4 + edv as i32) / 5;
        }
        self.sample_count += 1;
    }

    /// Update RSSI and compute SIR
    pub fn update_rssi(&mut self, rssi: i8) {
        self.rssi = rssi as i32;
        // SIR = RSSI - EDV (interference/noise floor)
        self.sir = self.rssi - self.edv;
    }

    /// Update SNR
    pub fn update_snr(&mut self, snr: i8) {
        self.snr = snr as i32;
    }

    /// Update packet error rate (0-100)
    pub fn update_per(&mut self, errors: u32, total: u32) {
        if total > 0 {
            self.per = ((errors * 100) / total) as i32;
        }
    }

    /// Get link quality score (0-100, higher is better)
    pub fn quality_score(&self) -> u32 {
        let rssi_score = ((self.rssi + 100).max(0).min(50) * 2) as u32;
        let sir_score = ((self.sir + 10).max(0).min(50) * 2) as u32;
        let per_score = (100 - self.per).max(0) as u32;

        // Weighted average: RSSI 30%, SIR 40%, PER 30%
        (rssi_score * 30 + sir_score * 40 + per_score * 30) / 100
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Radio link profile with uplink and downlink metrics
pub struct Profile<T> {
    pub inner: T,
    pub uplink_metrics: Metrics,   // Metrics for transmit direction
    pub downlink_metrics: Metrics, // Metrics for receive direction
}

impl<T> Profile<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            uplink_metrics: Metrics::new(),
            downlink_metrics: Metrics::new(),
        }
    }

    /// Get overall link quality (average of uplink and downlink)
    pub fn overall_quality(&self) -> u32 {
        (self.uplink_metrics.quality_score() + self.downlink_metrics.quality_score()) / 2
    }
}


pub mod profile;

use kaonic_radio::modulation::{
    Modulation, OfdmMcs, OfdmModulation, OfdmOption, QpskChipFrequency, QpskModulation,
    QpskRateMode,
};

/// Modulation scheme with specific parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulationScheme {
    Ofdm(OfdmModulation),
    Qpsk(QpskModulation),
}

impl ModulationScheme {
    /// Convert to kaonic_radio::Modulation
    pub fn to_modulation(&self) -> Modulation {
        match self {
            ModulationScheme::Ofdm(ofdm) => Modulation::Ofdm(*ofdm),
            ModulationScheme::Qpsk(qpsk) => Modulation::Qpsk(*qpsk),
        }
    }

    /// Get the type of modulation (OFDM or QPSK)
    pub fn modulation_type(&self) -> ModulationType {
        match self {
            ModulationScheme::Ofdm(_) => ModulationType::Ofdm,
            ModulationScheme::Qpsk(_) => ModulationType::Qpsk,
        }
    }

    /// Update transmit power
    pub fn with_tx_power(&self, power: u8) -> Self {
        match self {
            ModulationScheme::Ofdm(ofdm) => {
                let mut new_ofdm = *ofdm;
                new_ofdm.tx_power = power;
                ModulationScheme::Ofdm(new_ofdm)
            }
            ModulationScheme::Qpsk(qpsk) => {
                let mut new_qpsk = *qpsk;
                new_qpsk.tx_power = power;
                ModulationScheme::Qpsk(new_qpsk)
            }
        }
    }

    /// Adjust transmit power by a delta
    pub fn adjust_tx_power(&self, delta: i8) -> Self {
        match self {
            ModulationScheme::Ofdm(ofdm) => {
                let mut new_ofdm = *ofdm;
                new_ofdm.tx_power = (ofdm.tx_power as i16 + delta as i16)
                    .max(0)
                    .min(u8::MAX as i16) as u8;
                ModulationScheme::Ofdm(new_ofdm)
            }
            ModulationScheme::Qpsk(qpsk) => {
                let mut new_qpsk = *qpsk;
                new_qpsk.tx_power = (qpsk.tx_power as i16 + delta as i16)
                    .max(0)
                    .min(u8::MAX as i16) as u8;
                ModulationScheme::Qpsk(new_qpsk)
            }
        }
    }
}

/// Modulation type (without parameters)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulationType {
    Ofdm,
    Qpsk,
}

/// Channel quality assessment based on EDV measurements
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelQuality {
    Excellent, // Very low interference, EDV < -90 dBm
    Good,      // Low interference, EDV < -80 dBm
    Fair,      // Moderate interference, EDV < -70 dBm
    Poor,      // High interference, EDV < -60 dBm
    Bad,       // Very high interference, EDV >= -60 dBm
}

impl ChannelQuality {
    pub fn from_edv(edv: i8) -> Self {
        match edv {
            i8::MIN..=-70 => ChannelQuality::Excellent,
            -69..=-50 => ChannelQuality::Good,
            -49..=-30 => ChannelQuality::Fair,
            -29..=-10 => ChannelQuality::Poor,
            _ => ChannelQuality::Bad,
        }
    }

    /// Get recommended backoff time in milliseconds
    pub fn backoff_ms(&self) -> u32 {
        match self {
            ChannelQuality::Excellent => 1000,
            ChannelQuality::Good => 2000,
            ChannelQuality::Fair => 5000,
            ChannelQuality::Poor => 10000,
            ChannelQuality::Bad => 20000,
        }
    }

    /// Get recommended transmit power adjustment in dB
    pub fn tx_power_adjustment(&self) -> i8 {
        match self {
            ChannelQuality::Excellent => 0,
            ChannelQuality::Good => 0,
            ChannelQuality::Fair => 2,
            ChannelQuality::Poor => 4,
            ChannelQuality::Bad => 6,
        }
    }

    /// Get recommended OFDM modulation for this channel quality
    pub fn recommended_ofdm(&self, base_power: u8) -> OfdmModulation {
        match self {
            ChannelQuality::Excellent => OfdmModulation {
                mcs: OfdmMcs::Mcs6,       // Highest data rate (BPSK 1/2)
                opt: OfdmOption::Option1, // Smallest interleaving, fastest
                tx_power: base_power,
            },
            ChannelQuality::Good => OfdmModulation {
                mcs: OfdmMcs::Mcs4, // High data rate (QPSK 1/2)
                opt: OfdmOption::Option2,
                tx_power: base_power,
            },
            ChannelQuality::Fair => OfdmModulation {
                mcs: OfdmMcs::Mcs2,       // Medium data rate (QPSK 1/2)
                opt: OfdmOption::Option3, // More interleaving for robustness
                tx_power: base_power + 2,
            },
            ChannelQuality::Poor => OfdmModulation {
                mcs: OfdmMcs::Mcs1,       // Low data rate, more robust
                opt: OfdmOption::Option4, // Maximum interleaving
                tx_power: base_power + 4,
            },
            ChannelQuality::Bad => OfdmModulation {
                mcs: OfdmMcs::Mcs0,       // Lowest data rate, most robust
                opt: OfdmOption::Option4, // Maximum interleaving
                tx_power: base_power + 6,
            },
        }
    }

    /// Get recommended QPSK modulation for this channel quality
    pub fn recommended_qpsk(&self, base_power: u8) -> QpskModulation {
        match self {
            ChannelQuality::Excellent => QpskModulation {
                chip_freq: QpskChipFrequency::Freq2000, // Highest chip rate
                mode: QpskRateMode::Mode3,              // Highest data rate
                tx_power: base_power,
            },
            ChannelQuality::Good => QpskModulation {
                chip_freq: QpskChipFrequency::Freq1000,
                mode: QpskRateMode::Mode2,
                tx_power: base_power,
            },
            ChannelQuality::Fair => QpskModulation {
                chip_freq: QpskChipFrequency::Freq1000,
                mode: QpskRateMode::Mode1,
                tx_power: base_power + 2,
            },
            ChannelQuality::Poor => QpskModulation {
                chip_freq: QpskChipFrequency::Freq200,
                mode: QpskRateMode::Mode1,
                tx_power: base_power + 4,
            },
            ChannelQuality::Bad => QpskModulation {
                chip_freq: QpskChipFrequency::Freq100, // Lowest chip rate, most robust
                mode: QpskRateMode::Mode0,             // Lowest data rate
                tx_power: base_power + 6,
            },
        }
    }

    /// Get recommended modulation based on preferred modulation type
    pub fn recommended_modulation(
        &self,
        modulation_type: ModulationType,
        base_power: u8,
    ) -> ModulationScheme {
        match modulation_type {
            ModulationType::Ofdm => ModulationScheme::Ofdm(self.recommended_ofdm(base_power)),
            ModulationType::Qpsk => ModulationScheme::Qpsk(self.recommended_qpsk(base_power)),
        }
    }
}

/// EDV-based channel assessment
#[derive(Debug, Clone)]
pub struct ChannelAssessment {
    pub idle_edv: i8,           // EDV during idle state
    pub rx_edv: i8,             // EDV during RX state
    pub noise_floor: i8,        // Estimated noise floor
    pub interference_level: i8, // Estimated interference level
    pub quality: ChannelQuality,
    pub sample_count: u32,
    pub last_rx_time: Option<std::time::Instant>, // Time of last RX frame
    pub no_rx_timeout: std::time::Duration,       // Timeout to recover quality
}

impl ChannelAssessment {
    pub fn new() -> Self {
        Self {
            idle_edv: -127,
            rx_edv: -127,
            noise_floor: -127,
            interference_level: 0,
            quality: ChannelQuality::Excellent,
            sample_count: 0,
            last_rx_time: None,
            no_rx_timeout: std::time::Duration::from_secs(5), // Default 10 seconds
        }
    }

    /// Update assessment with new EDV reading
    pub fn update_idle(&mut self, edv: i8) {
        let old_quality = self.quality;
        self.sample_count += 1;

        // Use exponential moving average for smoothing
        if self.sample_count == 1 {
            self.idle_edv = edv;
            self.noise_floor = edv;
            log::debug!(
                "QoS: Initial idle EDV = {} dBm, noise floor = {} dBm",
                edv,
                self.noise_floor
            );
        } else {
            // EMA with alpha = 0.2
            self.idle_edv = ((self.idle_edv as i32 * 4 + edv as i32) / 5) as i8;
            self.noise_floor = self.idle_edv.min(self.noise_floor);
        }

        self.update_quality();

        if old_quality != self.quality {
            log::info!(
                "QoS: Channel quality changed {:?} -> {:?} (idle EDV: {} dBm, noise floor: {} dBm)",
                old_quality,
                self.quality,
                self.idle_edv,
                self.noise_floor
            );
        }
    }

    pub fn update_rx(&mut self, edv: i8) {
        let old_quality = self.quality;

        // Update last RX time
        self.last_rx_time = Some(std::time::Instant::now());

        // Use exponential moving average
        if self.sample_count == 0 {
            self.rx_edv = edv;
            log::debug!("QoS: Initial RX EDV = {} dBm", edv);
        } else {
            self.rx_edv = ((self.rx_edv as i32 * 4 + edv as i32) / 5) as i8;
        }

        // Interference is the difference between RX and noise floor
        self.interference_level = self.rx_edv.saturating_sub(self.noise_floor);

        self.update_quality();

        if old_quality != self.quality {
            log::info!(
                "QoS: Channel quality changed {:?} -> {:?} (RX EDV: {} dBm, interference: {} dB)",
                old_quality,
                self.quality,
                self.rx_edv,
                self.interference_level
            );
        } else if self.interference_level > 20 {
            log::debug!(
                "QoS: High interference detected (RX EDV: {} dBm, interference: {} dB)",
                self.rx_edv,
                self.interference_level
            );
        }
    }

    fn update_quality(&mut self) {
        // Use the higher (worse) EDV value for quality assessment
        let worst_edv = self.idle_edv.max(self.rx_edv);
        self.quality = ChannelQuality::from_edv(worst_edv);
    }

    /// Check if channel is clear for transmission (CCA)
    pub fn is_clear(&self, threshold: i8) -> bool {
        self.idle_edv < threshold
    }

    /// Get signal-to-interference ratio estimation
    pub fn get_sir_db(&self, signal_rssi: i8) -> i8 {
        signal_rssi.saturating_sub(self.rx_edv)
    }

    /// Check if we should recover channel quality due to no RX activity
    /// Returns true if quality was recovered
    pub fn check_no_rx_recovery(&mut self) -> bool {
        if let Some(last_rx) = self.last_rx_time {
            let elapsed = last_rx.elapsed();
            if elapsed > self.no_rx_timeout {
                let old_quality = self.quality;

                // If we haven't received anything, the interference might have cleared
                // Reset RX EDV to be closer to idle EDV
                self.rx_edv = ((self.rx_edv as i32 + self.idle_edv as i32 * 3) / 4) as i8;
                self.interference_level = self.rx_edv.saturating_sub(self.noise_floor);

                self.update_quality();

                if old_quality != self.quality {
                    log::info!(
                        "QoS: Channel quality recovered {:?} -> {:?} after {} s without RX (adjusted RX EDV: {} dBm)",
                        old_quality, self.quality, elapsed.as_secs(), self.rx_edv
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Set the timeout duration for no-RX quality recovery
    pub fn set_no_rx_timeout(&mut self, timeout: std::time::Duration) {
        self.no_rx_timeout = timeout;
        log::debug!(
            "QoS: No-RX recovery timeout set to {} seconds",
            timeout.as_secs()
        );
    }
}

impl Default for ChannelAssessment {
    fn default() -> Self {
        Self::new()
    }
}

/// QoS Manager with EDV-based channel assessment
pub struct QoSManager {
    assessment: ChannelAssessment,
    cca_threshold: i8, // Clear Channel Assessment threshold in dBm
    adaptive_tx_power: bool,
    adaptive_backoff: bool,
    adaptive_modulation: bool,
    modulation_type: ModulationType,
    default_modulation: ModulationScheme,
    base_tx_power: u8,
}

impl QoSManager {
    pub fn new() -> Self {
        log::debug!("QoS: Creating new QoS Manager with default settings");
        Self {
            assessment: ChannelAssessment::new(),
            cca_threshold: -75, // Default CCA threshold
            adaptive_tx_power: true,
            adaptive_backoff: true,
            adaptive_modulation: true,
            modulation_type: ModulationType::Ofdm,
            default_modulation: ModulationScheme::Ofdm(OfdmModulation {
                mcs: OfdmMcs::Mcs3,
                opt: OfdmOption::Option2,
                tx_power: 10,
            }),
            base_tx_power: 10,
        }
    }

    pub fn with_cca_threshold(mut self, threshold: i8) -> Self {
        log::debug!("QoS: Setting CCA threshold to {} dBm", threshold);
        self.cca_threshold = threshold;
        self
    }

    pub fn enable_adaptive_tx_power(mut self, enabled: bool) -> Self {
        log::debug!(
            "QoS: Adaptive TX power: {}",
            if enabled { "enabled" } else { "disabled" }
        );
        self.adaptive_tx_power = enabled;
        self
    }

    pub fn enable_adaptive_backoff(mut self, enabled: bool) -> Self {
        log::debug!(
            "QoS: Adaptive backoff: {}",
            if enabled { "enabled" } else { "disabled" }
        );
        self.adaptive_backoff = enabled;
        self
    }

    pub fn enable_adaptive_modulation(mut self, enabled: bool) -> Self {
        log::debug!(
            "QoS: Adaptive modulation: {}",
            if enabled { "enabled" } else { "disabled" }
        );
        self.adaptive_modulation = enabled;
        self
    }

    pub fn with_modulation_type(mut self, modulation_type: ModulationType) -> Self {
        log::debug!("QoS: Setting modulation type to {:?}", modulation_type);
        self.modulation_type = modulation_type;
        self
    }

    pub fn with_default_modulation(mut self, modulation: ModulationScheme) -> Self {
        log::debug!("QoS: Setting default modulation to {:?}", modulation);
        self.default_modulation = modulation;
        self.modulation_type = modulation.modulation_type();
        self.base_tx_power = match modulation {
            ModulationScheme::Ofdm(ofdm) => ofdm.tx_power,
            ModulationScheme::Qpsk(qpsk) => qpsk.tx_power,
        };
        self
    }

    pub fn with_base_tx_power(mut self, power: u8) -> Self {
        self.base_tx_power = power;
        self
    }

    pub fn with_no_rx_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.assessment.set_no_rx_timeout(timeout);
        self
    }

    /// Update with EDV reading during idle state
    pub fn update_idle_edv(&mut self, edv: i8) {
        self.assessment.update_idle(edv);

        // Check if we should recover quality due to no RX activity
        self.assessment.check_no_rx_recovery();
    }

    /// Update with EDV reading during RX state
    pub fn update_rx_edv(&mut self, edv: i8) {
        self.assessment.update_rx(edv);
    }

    /// Get current channel assessment
    pub fn get_assessment(&self) -> &ChannelAssessment {
        &self.assessment
    }

    /// Check if channel is clear for transmission
    pub fn can_transmit(&self) -> bool {
        self.assessment.is_clear(self.cca_threshold)
    }

    /// Get recommended backoff time before retry
    pub fn get_backoff_ms(&self) -> u32 {
        if self.adaptive_backoff {
            self.assessment.quality.backoff_ms()
        } else {
            0
        }
    }

    /// Get recommended transmit power adjustment
    pub fn get_tx_power_adjustment(&self) -> i8 {
        if self.adaptive_tx_power {
            self.assessment.quality.tx_power_adjustment()
        } else {
            0
        }
    }

    /// Get recommended modulation based on current channel quality
    pub fn get_recommended_modulation(&self) -> ModulationScheme {
        if self.adaptive_modulation {
            let modulation = self
                .assessment
                .quality
                .recommended_modulation(self.modulation_type, self.base_tx_power);
            log::trace!(
                "QoS: Recommended modulation for {:?} quality: {:?}",
                self.assessment.quality,
                modulation
            );
            modulation
        } else {
            self.default_modulation
        }
    }

    /// Get modulation as kaonic_radio::Modulation
    pub fn get_modulation(&self) -> Modulation {
        self.get_recommended_modulation().to_modulation()
    }

    /// Get recommended OFDM modulation
    pub fn get_recommended_ofdm(&self) -> OfdmModulation {
        self.assessment.quality.recommended_ofdm(self.base_tx_power)
    }

    /// Get recommended QPSK modulation
    pub fn get_recommended_qpsk(&self) -> QpskModulation {
        self.assessment.quality.recommended_qpsk(self.base_tx_power)
    }

    /// Reset statistics
    pub fn reset(&mut self) {
        log::debug!("QoS: Resetting channel assessment statistics");
        self.assessment = ChannelAssessment::new();
    }
}

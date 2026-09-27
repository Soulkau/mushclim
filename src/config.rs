use embassy_time::Duration;
use serde::{Deserialize, Serialize};

use crate::{exhaust::ExhaustConfig, humidifier::HumidifierConfig};

/// Dto of `MushclimConfig`.
/// As `MushclimConfig` uses embassy and other local types for ergonomics, so it cannot be serialized easily.
///
/// NOTE: All of duration values in config use seconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MushclimConfigDto {
    /// Currently unsuported, field that used for ota config updates, defines if reset is *hard or *soft.
    pub preserve: bool,
    /// Lower bound of humidity threshold, e.g. 86%
    pub humidity_lower_bound: u16,
    /// Upper bound of humidity threshold, e.g. 90%
    pub humidity_upper_bound: u16,
    /// Retry count on any error
    pub retry_count: u16,
    /// Maximum temperature delta over single tick
    pub max_temp_delta: u16,
    /// Maximum humidity delta over single tick
    pub max_humidity_delta: u16,
    /// Number of calibration samples
    pub calibration_samples: usize,
    /// Lower bound of CO2 ppm threshold
    pub co2ppm_lower_bound: u16,
    /// Upper bound of CO2 ppm threshold
    pub co2ppm_upper_bound: u16,
    /// Minimum time to wait before the exhaust fan can be turned on again after being turned off
    pub exhaust_cooldown: u64,
    /// Maximum time for exhaust to work, trying to hit co2ppm_lower_bound before going into cooldown.
    pub exhaust_timeout: u64,
    /// Tick/loop frequency
    pub loop_delay: u64,
}

impl Default for MushclimConfigDto {
    fn default() -> Self {
        Self {
            preserve: true,
            humidity_lower_bound: 86,
            humidity_upper_bound: 90,
            retry_count: 10,
            max_temp_delta: 9,
            max_humidity_delta: 30,
            calibration_samples: 10,
            co2ppm_lower_bound: 700,
            co2ppm_upper_bound: 1000,
            exhaust_cooldown: 5 * 60,
            exhaust_timeout: 60 * 3,
            loop_delay: 30,
        }
    }
}

/// Config structure that is used across whole app.
///
/// IMPORTANT: All durations are in seconds.
#[derive(Debug)]
pub struct MushclimConfig {
    pub humidifier: HumidifierConfig,
    pub exhaust: ExhaustConfig,
    pub loop_delay: Duration,
    pub retry_count: usize,
}

impl MushclimConfigDto {
    /// Converts the DTO into the actual runtime `MushclimConfig`.
    pub fn as_local(&self) -> MushclimConfig {
        MushclimConfig {
            humidifier: self.into(),
            exhaust: self.into(),
            loop_delay: Duration::from_secs(self.loop_delay),
            retry_count: self.retry_count as usize,
        }
    }
}

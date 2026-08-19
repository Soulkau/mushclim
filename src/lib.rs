#![no_std]

use core::ops::RangeInclusive;
use embassy_time::Duration;
use serde::{Deserialize, Serialize};

pub mod exhaust;
pub mod humidifier;
pub mod lcd;
pub mod measurements;
pub mod metrics;
pub mod timer;
pub mod wifi;
extern crate alloc;

#[macro_export]
macro_rules! arcmutex {
    ($val:expr) => {{
        use alloc::sync::Arc;
        use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

        Arc::new(Mutex::<CriticalSectionRawMutex, _>::new($val))
    }};
}

#[macro_export]
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        STATIC_CELL.init(($val))
    }};
}

pub const METRIC_SNAPSHOT_AMOUNT: usize = 10;

#[derive(Debug)]
pub struct MushclimConfig {
    pub humidity_threshold: RangeInclusive<u16>, //Humidity threshold, lower 86% - upper 90%
    pub retry_count: usize,                      //Retry count on any error
    pub max_temp_delta: u16,                     //Maximum temperature delta over single tick
    pub max_humidity_delta: u16,                 //Maximum humidity delta over single tick
    pub calibration_samples: usize,              //Number of calibration samples
    pub safe_humidity_duty_interval: Duration, //Interval between humidity duty cycles in safe mode
    pub safe_humidity_duty: Duration,          //Duration of humidity duty cycle in safe mode
    pub exhaust_duty: Duration,                //Exhaust duty duration
    pub exhaust_duty_interval: Duration,       //Interval between exhaust duty cycles
    pub loop_delay: Duration,                  //Tick/loop frequency
    pub metric_send_period: Duration,
}
impl Default for MushclimConfig {
    fn default() -> Self {
        Self {
            humidity_threshold: 86..=90,
            retry_count: 10,
            max_temp_delta: 9,
            max_humidity_delta: 30,
            calibration_samples: 10,
            safe_humidity_duty_interval: Duration::from_secs(20 * 60),
            safe_humidity_duty: Duration::from_secs(4 * 60),
            exhaust_duty: Duration::from_secs(120),
            exhaust_duty_interval: Duration::from_secs(50 * 60),
            loop_delay: Duration::from_secs(30),
            metric_send_period: Duration::from_secs(3600),
        }
    }
}

/// All of duration values in config use seconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MushclimConfigDto {
    #[serde(default = "default_true")]
    pub preserve: bool,
    pub humidity_lower_bound: u16,
    pub humidity_upper_bound: u16,
    pub retry_count: usize,
    pub max_temp_delta: u16,
    pub max_humidity_delta: u16,
    pub calibration_samples: usize,
    pub safe_humidity_duty_interval: u64,
    pub safe_humidity_duty: u64,
    pub exhaust_duty: u64,
    pub exhaust_duty_interval: u64,
    pub loop_delay: u64,
    pub metric_send_period: u64,
}

fn default_true() -> bool {
    true
}

impl MushclimConfigDto {
    /// Converts the DTO into the actual runtime `MushclimConfig`.
    pub fn as_local(&self) -> MushclimConfig {
        MushclimConfig {
            humidity_threshold: self.humidity_lower_bound..=self.humidity_upper_bound,
            retry_count: self.retry_count,
            max_temp_delta: self.max_temp_delta,
            max_humidity_delta: self.max_humidity_delta,
            calibration_samples: self.calibration_samples,
            safe_humidity_duty_interval: Duration::from_secs(self.safe_humidity_duty_interval),
            safe_humidity_duty: Duration::from_secs(self.safe_humidity_duty),
            exhaust_duty: Duration::from_secs(self.exhaust_duty),
            exhaust_duty_interval: Duration::from_secs(self.exhaust_duty_interval),
            loop_delay: Duration::from_secs(self.loop_delay),
            metric_send_period: Duration::from_secs(self.metric_send_period),
        }
    }
}

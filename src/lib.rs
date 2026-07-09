#![no_std]

use core::ops::RangeInclusive;

pub mod exhaust;
pub mod lcd;
pub mod measurements;

#[macro_export]
macro_rules! arcmutex {
    ($val:expr) => {{
        use alloc::sync::Arc;
        use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

        Arc::new(Mutex::<CriticalSectionRawMutex, _>::new($val))
    }};
}

pub struct MushclimConfig {
    pub humidity_threshold: RangeInclusive<u16>,
    pub retry_count: usize,
    pub max_temp_delta: u16,
    pub max_humidity_delta: u16,
    pub calibration_samples: usize,
    pub safe_mode_sleep: u64,
    pub safe_humidity_duty_cycle: u64,
    pub exhaust_duty_cycle: u64,
    pub exhaust_duty_interval: u64,
}

impl Default for MushclimConfig {
    fn default() -> Self {
        Self {
            // Lowest bound is 80%, comfort target bound is 88%
            humidity_threshold: 80..=90,
            retry_count: 5,
            max_temp_delta: 5,
            max_humidity_delta: 10,
            calibration_samples: 5,
            safe_mode_sleep: 600,
            safe_humidity_duty_cycle: 60,
            exhaust_duty_cycle: 2,
            exhaust_duty_interval: 30,
        }
    }
}

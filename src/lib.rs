#![no_std]

use core::ops::RangeInclusive;
use embassy_time::Duration;

pub mod exhaust;
pub mod humidifier;
pub mod lcd;
pub mod measurements;
pub mod mqtt;

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

pub struct MushclimConfig {
    pub humidity_threshold: RangeInclusive<u16>,
    pub retry_count: usize,
    pub max_temp_delta: u16,
    pub max_humidity_delta: u16,
    pub calibration_samples: usize,
    pub safe_humidity_duty_interval: Duration,
    pub safe_humidity_duty_cycle: Duration,
    pub exhaust_duty_cycle: Duration,
    pub exhaust_duty_interval: Duration,
    pub loop_delay: Duration,
}

impl Default for MushclimConfig {
    fn default() -> Self {
        Self {
            // Lowest bound is 80%, comfort target bound is 88%
            humidity_threshold: 86..=90,
            retry_count: 10,
            max_temp_delta: 9,
            max_humidity_delta: 30,
            calibration_samples: 10,
            safe_humidity_duty_interval: Duration::from_secs(20 * 60),
            safe_humidity_duty_cycle: Duration::from_secs(4 * 60),
            exhaust_duty_cycle: Duration::from_secs(120),
            exhaust_duty_interval: Duration::from_secs(50 * 60),
            loop_delay: Duration::from_secs(30),
        }
    }
}

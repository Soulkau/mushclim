#![no_std]

use core::ops::RangeInclusive;
use embassy_time::Duration;

pub mod cycle;
pub mod exhaust;
pub mod humidifier;
pub mod lcd;
pub mod measurements;

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
        }
    }
}

#![no_std]

use core::ops::RangeInclusive;
use embassy_time::Duration;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, i2c::I2c};
use embedded_storage::nor_flash::NorFlash;
use serde::{Deserialize, Serialize};

pub mod app;
pub mod exhaust;
pub mod humidifier;
pub mod measurements;
pub mod metrics;
pub mod timer;
extern crate alloc;

#[derive(Debug, Serialize)]
pub struct MushclimStats {
    pub temperature: i16,
    pub humidity: u16,
    pub humidifier_on: bool,
    pub exhaust_on: bool,
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

/// All of duration values in config use seconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MushclimConfigDto {
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
            safe_humidity_duty_interval: 20 * 60,
            safe_humidity_duty: 4 * 60,
            exhaust_duty: 60,
            exhaust_duty_interval: 30 * 60,
            loop_delay: 30,
            metric_send_period: 3600,
        }
    }
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

pub trait MushclimPlatform {
    type ExhaustPin: OutputPin;
    type LightPin: OutputPin;
    type DiscoPin: OutputPin;
    type HumidifierPin: OutputPin;
    type FlashStorage: NorFlash + 'static;
    type Sensor: I2c;
    type Delay: DelayNs;
}

#![no_std]

use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, i2c::I2c};
use embedded_storage::nor_flash::NorFlash;
use serde::Serialize;

pub mod app;
pub mod config;
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
    pub co2ppm: u16,
    pub humidifier_on: bool,
    pub exhaust_on: bool,
}

pub const METRIC_SNAPSHOT_AMOUNT: usize = 10;

/// Used by chips to provide support for mushclim.
pub trait MushclimPlatform {
    type ExhaustPin: OutputPin;
    type LightPin: OutputPin;
    // Blue led lights, optional
    type DiscoPin: OutputPin;
    type HumidifierPin: OutputPin;
    // Persistent(nvs) flash region
    type NvsStorage: NorFlash + 'static;
    // Stcc4
    type Sensor: I2c;
    type Delay: DelayNs;
}

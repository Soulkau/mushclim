#![no_std]

use serde::Serialize;

pub mod app;
pub mod config;
pub mod exhaust;
mod heater;
mod humidifier;
mod measurements;
mod timer;
extern crate alloc;

pub use ivy;

#[derive(Debug, Serialize)]
pub struct MushclimStatus {
    pub temperature: i16,
    pub humidity: u16,
    pub co2ppm: u16,
    pub humidifier_on: bool,
    pub exhaust_on: bool,
}

pub const METRIC_SNAPSHOT_AMOUNT: usize = 10;

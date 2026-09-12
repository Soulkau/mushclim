use core::fmt::Debug;

use driverse::stcc4::{self, Stcc4};
use embedded_hal_async::{delay::DelayNs, i2c::I2c};

#[derive(thiserror::Error, Debug)]
pub(crate) enum MeasurementError {
    #[error("sensor communication failed")]
    Sensor,
    #[error("sensor crc mismatch")]
    Crc,
}

impl<E: Debug> From<stcc4::Error<E>> for MeasurementError {
    fn from(err: stcc4::Error<E>) -> Self {
        match err {
            stcc4::Error::Crc => Self::Crc,
            _ => Self::Sensor,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Measurement {
    pub co2_ppm: u16,
    pub temperature_c: f32,
    pub humidity_pct: f32,
}

impl From<stcc4::Measurement> for Measurement {
    fn from(m: stcc4::Measurement) -> Self {
        Self {
            co2_ppm: m.co2,
            temperature_c: m.temperature,
            humidity_pct: m.humidity,
        }
    }
}

pub(crate) struct MeasurementManager<I: I2c, D: DelayNs> {
    sensor: Stcc4<I, D>,
}

impl<I: I2c, D: DelayNs> MeasurementManager<I, D> {
    pub fn new(sensor: Stcc4<I, D>) -> Self {
        Self { sensor }
    }

    pub async fn init(&mut self) {
        self.sensor.init().await.ok();
        self.sensor.perform_conditioning().await.ok();
    }

    pub async fn measure(&mut self) -> Result<Measurement, MeasurementError> {
        let sm = self.sensor.measure().await?;

        Ok(sm.into())
    }
}

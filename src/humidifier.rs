use core::ops::RangeInclusive;

use driverse::relay::Relay;
use embedded_hal::digital::OutputPin;

use crate::{
    config::{MushclimConfig, MushclimConfigDto},
    measurements::Measurement,
};

#[derive(Debug, Clone)]
pub struct HumidifierConfig {
    pub humidity_threshold: RangeInclusive<u16>,
}

impl From<&MushclimConfigDto> for HumidifierConfig {
    fn from(dto: &MushclimConfigDto) -> Self {
        Self {
            humidity_threshold: dto.humidity_lower_bound..=dto.humidity_upper_bound,
        }
    }
}

pub(crate) struct Humidifier<P: OutputPin> {
    switch: Relay<P>,
    config: HumidifierConfig,
}

impl<P: OutputPin> Humidifier<P> {
    pub fn new(switch: Relay<P>, config: &HumidifierConfig) -> Self {
        Self {
            switch,
            config: config.clone(),
        }
    }

    /// Tick, uses live humidity measurements.
    pub fn tick(&mut self, measurements: &Measurement, is_exhaust_on: bool) {
        if self.pause_for_exhaust(is_exhaust_on) {
            return;
        }

        let current_humidity = measurements.humidity_pct;
        let low_bound = *self.config.humidity_threshold.start() as f32;
        let comfort_bound = *self.config.humidity_threshold.end() as f32;

        if current_humidity <= low_bound {
            if !self.switch.is_on() {
                tracing::info!(
                    tag = "humidifier",
                    "humidity ({}%) below low bound ({}%). Turning humidifier ON.",
                    current_humidity,
                    low_bound
                );
                self.switch.on().ok();
            }
        } else if current_humidity >= comfort_bound {
            if self.switch.is_on() {
                tracing::info!(
                    tag = "humidifier",
                    "humidity ({}%) reached comfort bound ({}%). Turning humidifier OFF.",
                    current_humidity,
                    comfort_bound
                );
                self.switch.off().ok();
            }
        }
    }

    fn pause_for_exhaust(&mut self, is_exhaust_on: bool) -> bool {
        if is_exhaust_on {
            if self.switch.is_on() {
                tracing::info!(
                    tag = "humidifier",
                    "paused humidifer due to exhaust being on"
                );
            }
            self.switch.off().ok();
            return true;
        }
        false
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.config = config.humidifier.clone();
    }

    pub fn off(&mut self) {
        let _ = self.switch.off();
    }

    pub fn is_on(&self) -> bool {
        self.switch.is_on()
    }
}

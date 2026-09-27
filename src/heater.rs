use core::ops::RangeInclusive;

use driverse::relay::Relay;
use embedded_hal::digital::OutputPin;

use crate::{
    config::{MushclimConfig, MushclimConfigDto},
    measurements::Measurement,
};

#[derive(Debug, Clone)]
pub struct HeaterConfig {
    pub threshold: RangeInclusive<u8>,
}

impl From<&MushclimConfigDto> for HeaterConfig {
    fn from(dto: &MushclimConfigDto) -> Self {
        Self {
            threshold: dto.temperature_lower_bound..=dto.temperature_upper_bound,
        }
    }
}

pub(crate) struct Heater<P: OutputPin> {
    relay: Relay<P>,
    config: HeaterConfig,
}

impl<P: OutputPin> Heater<P> {
    pub fn new(relay: Relay<P>, config: &HeaterConfig) -> Self {
        Self {
            relay,
            config: config.clone(),
        }
    }

    /// Tick, uses live temperature measurements. Can't be interrupted (no exhaust pause) —
    /// just runs until bounds are met.
    pub fn tick(&mut self, measurements: &Measurement) {
        let current_temp = measurements.temperature_c;
        let low_bound = *self.config.threshold.start() as f32;
        let comfort_bound = *self.config.threshold.end() as f32;

        if current_temp <= low_bound {
            if !self.relay.is_on() {
                tracing::info!(
                    tag = "heater",
                    "temperature ({}°C) below low bound ({}°C). heater on.",
                    current_temp,
                    low_bound
                );
                self.relay.on().ok();
            }
        } else if current_temp >= comfort_bound {
            if self.relay.is_on() {
                tracing::info!(
                    tag = "heater",
                    "temperature ({}°C) reached comfort bound ({}°C). heater off.",
                    current_temp,
                    comfort_bound
                );
                self.relay.off().ok();
            }
        }
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.config = config.heater.clone();
    }

    pub fn off(&mut self) {
        let _ = self.relay.off();
    }
}

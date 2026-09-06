use core::ops::RangeInclusive;

use driverse::relay::Relay;
use embedded_hal::digital::OutputPin;

use crate::{MushclimConfig, measurements::Measurement};

pub(crate) struct Humidifier<P: OutputPin> {
    switch: Relay<P>,
    treshold: RangeInclusive<u16>,
}

impl<P: OutputPin> Humidifier<P> {
    pub fn new(switch: Relay<P>, config: &MushclimConfig) -> Self {
        Self {
            switch,
            treshold: config.humidity_threshold.clone(),
        }
    }

    pub fn tick(&mut self, measurements: &Measurement, is_exhaust_on: bool) {
        if is_exhaust_on {
            if self.switch.is_on() {
                tracing::info!("Humidifier: Paused, exhaust active.");
            }
            self.switch.off().ok();
            return;
        }

        let current_humidity = measurements.humidity_pct;
        let low_bound = *self.treshold.start() as f32;
        let comfort_bound = *self.treshold.end() as f32;

        if current_humidity <= low_bound {
            if !self.switch.is_on() {
                tracing::info!(
                    "Humidity ({}%) below low bound ({}%). Turning humidifier ON.",
                    current_humidity,
                    low_bound
                );
                self.switch.on().ok();
            }
        } else if current_humidity >= comfort_bound {
            if self.switch.is_on() {
                tracing::info!(
                    "Humidity ({}%) reached comfort bound ({}%). Turning humidifier OFF.",
                    current_humidity,
                    comfort_bound
                );
                self.switch.off().ok();
            }
        }
    }

    pub fn update_config(&mut self, config: &MushclimConfig) {
        self.treshold = config.humidity_threshold.clone();
    }

    pub fn turn_on(&mut self) {
        let _ = self.switch.on();
    }

    pub fn turn_off(&mut self) {
        let _ = self.switch.off();
    }

    pub fn is_on(&self) -> bool {
        self.switch.is_on()
    }
}

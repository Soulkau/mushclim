use core::ops::RangeInclusive;

use driverse::relay::Relay;
use esp_hal::gpio::Output;

use crate::{MushclimConfig, measurements::Measurements};

pub struct Humidifier<'a> {
    switch: Relay<Output<'a>>,
    treshold: RangeInclusive<u16>,
}

impl<'a> Humidifier<'a> {
    pub fn new(switch: Relay<Output<'a>>, config: &MushclimConfig) -> Self {
        Self {
            switch,
            treshold: config.humidity_threshold.clone(),
        }
    }

    pub fn tick(&mut self, measurements: &Measurements, is_exhaust_on: bool) {
        if is_exhaust_on {
            if self.switch.is_on() {
                tracing::info!("Humidifier: Paused, exhaust active.");
            }
            self.switch.off();
            return;
        }

        let current_humidity = measurements.humidity;
        let low_bound = *self.treshold.start();
        let comfort_bound = *self.treshold.end();

        if current_humidity <= low_bound {
            if !self.switch.is_on() {
                tracing::info!(
                    "Humidity ({}%) below low bound ({}%). Turning humidifier ON.",
                    current_humidity,
                    low_bound
                );
                let _ = self.switch.on();
            }
        } else if current_humidity >= comfort_bound {
            if self.switch.is_on() {
                tracing::info!(
                    "Humidity ({}%) reached comfort bound ({}%). Turning humidifier OFF.",
                    current_humidity,
                    comfort_bound
                );
                let _ = self.switch.off();
            }
        }
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

use core::ops::RangeInclusive;

use driverse::relay::Relay;
use embassy_time::Duration;
use embedded_hal::digital::OutputPin;

use crate::{
    config::{MushclimConfig, MushclimConfigDto},
    measurements::Measurement,
    timer::CycleTimer,
};

#[derive(Debug, Clone)]
pub struct HumidifierConfig {
    pub humidity_threshold: RangeInclusive<u16>,
    pub safe_humidity_duty_interval: Duration,
    pub safe_humidity_duty: Duration,
}

impl From<&MushclimConfigDto> for HumidifierConfig {
    fn from(dto: &MushclimConfigDto) -> Self {
        Self {
            humidity_threshold: dto.humidity_lower_bound..=dto.humidity_upper_bound,
            safe_humidity_duty_interval: Duration::from_secs(dto.safe_humidity_duty_interval),
            safe_humidity_duty: Duration::from_secs(dto.safe_humidity_duty),
        }
    }
}

pub enum HumidifierMode {
    Normal,
    Cycled(CycleTimer),
}

pub(crate) struct Humidifier<P: OutputPin> {
    switch: Relay<P>,
    config: HumidifierConfig,
    mode: HumidifierMode,
}

impl<P: OutputPin> Humidifier<P> {
    pub fn new(switch: Relay<P>, config: &HumidifierConfig) -> Self {
        Self {
            switch,
            config: config.clone(),
            mode: HumidifierMode::Normal,
        }
    }

    pub fn set_normal_mode(&mut self) {
        tracing::info!("[Humidifier]: switching to normal mode");
        self.mode = HumidifierMode::Normal;
        self.turn_off();
    }

    pub fn set_cycled_mode(&mut self) {
        tracing::info!("[Humidifier]: switching to cycled mode");
        self.mode = HumidifierMode::Cycled(CycleTimer::new(
            self.config.safe_humidity_duty_interval,
            self.config.safe_humidity_duty,
        ));
        self.turn_off();
    }

    /// Normal-mode tick, uses live humidity measurements. No-op if currently in cycled mode.
    pub fn tick(&mut self, measurements: &Measurement, is_exhaust_on: bool) {
        if !matches!(self.mode, HumidifierMode::Normal) {
            tracing::debug!("[Humidifier]: tick() called while in cycled mode, ignoring");
            return;
        }

        if self.pause_for_exhaust(is_exhaust_on) {
            return;
        }

        let current_humidity = measurements.humidity_pct;
        let low_bound = *self.config.humidity_threshold.start() as f32;
        let comfort_bound = *self.config.humidity_threshold.end() as f32;

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

    /// Cycled-mode tick, no sensor needed. No-op if currently in normal mode.
    pub fn tick_cycled(&mut self, is_exhaust_on: bool) {
        // Return if in wrong mode
        if matches!(self.mode, HumidifierMode::Normal) {
            tracing::debug!("[Humidifier]: tick_cycled() called while in normal mode, ignoring");
            return;
        }
        // Ignore tick if exhaust is on.
        if self.pause_for_exhaust(is_exhaust_on) {
            if let HumidifierMode::Cycled(timer) = &mut self.mode {
                timer.ingore_tick();
            }
            return;
        }

        let should_be_on = if let HumidifierMode::Cycled(timer) = &mut self.mode {
            timer.tick()
        } else {
            false
        };

        if should_be_on && !self.switch.is_on() {
            tracing::info!("[Humidifier]: cycled duty started, turning ON.");
            self.switch.on().ok();
        } else if !should_be_on && self.switch.is_on() {
            tracing::info!("[Humidifier]: cycled duty ended, turning OFF.");
            self.switch.off().ok();
        }
    }

    fn pause_for_exhaust(&mut self, is_exhaust_on: bool) -> bool {
        if is_exhaust_on {
            if self.switch.is_on() {
                tracing::info!("[Humidifer]: Paused humidifer due to exhaust being on");
            }
            self.switch.off().ok();
            return true;
        }
        false
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.config = config.humidifier.clone();
        if matches!(self.mode, HumidifierMode::Cycled(_)) {
            // duty/interval may have changed, rebuild the timer against the new config
            self.mode = HumidifierMode::Cycled(CycleTimer::new(
                self.config.safe_humidity_duty_interval,
                self.config.safe_humidity_duty,
            ));
        }
    }

    pub fn turn_off(&mut self) {
        let _ = self.switch.off();
    }

    pub fn is_on(&self) -> bool {
        self.switch.is_on()
    }
}

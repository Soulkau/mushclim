use crate::{
    MushclimConfig,
    timer::{CycleTimer, DurationExts},
};
use core::fmt::Write;
use driverse::relay::Relay;
use embedded_hal::digital::OutputPin;
use heapless::String;

pub struct ExhaustManager<P: OutputPin> {
    pub exhaust: Relay<P>,
    pub timer: CycleTimer,
}

impl<P: OutputPin> ExhaustManager<P> {
    pub fn new(exhaust: Relay<P>, config: &MushclimConfig) -> Self {
        Self {
            exhaust,
            timer: CycleTimer::new(config.exhaust_duty_interval, config.exhaust_duty),
        }
    }

    pub async fn tick(&mut self) -> bool {
        let should_be_on = self.timer.tick();
        let is_on = self.exhaust.is_on();
        if should_be_on {
            if !is_on {
                self.exhaust.on().ok();
                tracing::info!("[Exhaust]: Turned on");
            }
        } else {
            if is_on {
                self.exhaust.off().ok();
                tracing::info!("[Exhaust]: Turned off");
            }
        }
        should_be_on
    }

    pub fn state_log(&self) -> String<32> {
        let (is_on_duty, until_switch) = self.timer.time_until_change();
        let state = if is_on_duty { "off" } else { "on" };
        let mut s = String::new();

        if write!(
            s,
            "on: {} / {} in {}",
            is_on_duty,
            state,
            until_switch.pretty_string()
        )
        .is_err()
        {
            s.clear();
            let _ = s.push_str("fan state fmt err");
        }

        s
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.timer = CycleTimer::new(config.exhaust_duty_interval, config.exhaust_duty);
    }

    pub fn reset(&mut self) {
        self.exhaust.off().ok();
    }
}

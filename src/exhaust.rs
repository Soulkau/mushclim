use driverse::relay::Relay;
use embedded_hal::digital::OutputPin;

use crate::{MushclimConfig, timer::CycleTimer};

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

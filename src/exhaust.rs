use core::fmt::Write;

use driverse::relay::Relay;
use esp_hal::gpio::Output;
use heapless::String;
use ivy::mqtt::MqttHandle;
use serde::{Deserialize, Serialize};

use crate::{MushclimConfig, timer::CycleTimer};

pub struct ExhaustManager<'a> {
    pub exhaust: Relay<Output<'a>>,
    pub timer: CycleTimer,
    pub handle: MqttHandle<255>,
}

impl<'a> ExhaustManager<'a> {
    pub fn new(
        exhaust: Relay<Output<'a>>,
        config: &MushclimConfig,
        handle: MqttHandle<255>,
    ) -> Self {
        Self {
            exhaust,
            timer: CycleTimer::new(config.exhaust_duty_interval, config.exhaust_duty),
            handle,
        }
    }

    pub async fn tick(&mut self) -> bool {
        let should_be_on = self.timer.tick();
        let is_on = self.exhaust.is_on();
        if should_be_on {
            if !is_on {
                self.switch(true).await;
                tracing::info!("[Exhaust]: Turned on");
            }
        } else {
            if is_on {
                self.switch(false).await;
                tracing::info!("[Exhaust]: Turned off");
            }
        }
        should_be_on
    }

    pub async fn switch(&mut self, on: bool) {
        self.handle
            .publish("mushclim/fan", ExhaustState { on })
            .await;
        if on {
            self.exhaust.on();
        } else {
            self.exhaust.off();
        }
    }

    pub fn log_state(&self) -> String<16> {
        let mut string = String::new();
        let _ = string.write_str("Fan: ");
        let (started, time) = self.timer.time_until_change();

        if started {
            let _ = string.write_str("off");
        } else {
            let _ = string.write_str("on");
        }
        let _ = write!(string, " in {}", time);
        string
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.timer = CycleTimer::new(config.exhaust_duty_interval, config.exhaust_duty);
    }

    pub fn reset(&mut self) {
        self.exhaust.off();
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ExhaustState {
    on: bool,
}

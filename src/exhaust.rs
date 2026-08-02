use core::{fmt::Write, ops::Sub};

use driverse::relay::Relay;
use embassy_time::{Duration, Instant};
use esp_hal::gpio::Output;
use heapless::String;

use crate::{MushclimConfig, cycle::CycleTimer};

pub struct ExhaustManager<'a> {
    pub exhaust: Relay<Output<'a>>,
    pub timer: CycleTimer,
}

impl<'a> ExhaustManager<'a> {
    pub fn new(exhaust: Relay<Output<'a>>, config: &MushclimConfig) -> Self {
        Self {
            exhaust,
            timer: CycleTimer::new(config.exhaust_duty_interval, config.exhaust_work),
        }
    }

    pub async fn tick(&mut self) -> bool {
        let should_be_on = self.timer.tick();

        if should_be_on {
            if !self.exhaust.is_on() {
                defmt::info!("ExhaustManager: Starting fresh air exchange window.");
            }
            self.exhaust.on();
        } else {
            if self.exhaust.is_on() {
                defmt::info!("ExhaustManager: Ending fresh air exchange window.");
            }
            self.exhaust.off();
        }
        should_be_on
    }

    pub fn format_state(&self) -> String<16> {
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

    pub fn reset(&mut self) {
        self.exhaust.off();
    }
}

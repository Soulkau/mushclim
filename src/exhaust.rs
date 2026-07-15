use driverse::relay::Relay;
use esp_hal::gpio::Output;

use crate::MushclimConfig;

pub struct ExhaustManager<'a> {
    pub exhaust: Relay<Output<'a>>,
    pub duty_cycle: u64,
    pub duty_interval: u64,
    pub minutes_elapsed: u64,
}

impl<'a> ExhaustManager<'a> {
    pub fn new(exhaust: Relay<Output<'a>>, config: &MushclimConfig) -> Self {
        Self {
            exhaust,
            duty_cycle: config.exhaust_duty_cycle,
            duty_interval: config.exhaust_duty_interval,
            minutes_elapsed: 0,
        }
    }

    pub async fn tick(&mut self) -> bool {
        let minute_in_cycle = self.minutes_elapsed % self.duty_interval;
        let burst_start = self.duty_interval.saturating_sub(self.duty_cycle);
        let is_turned_on = minute_in_cycle >= burst_start;
        if is_turned_on {
            if !self.exhaust.is_on() {
                log::info!("ExhaustManager: Starting fresh air exchange window.");
            }
            self.exhaust.on();
        } else {
            if self.exhaust.is_on() {
                log::info!("ExhaustManager: Ending fresh air exchange window.");
            }
            self.exhaust.off();
        }
        self.minutes_elapsed += 1;
        if self.minutes_elapsed >= self.duty_interval {
            self.minutes_elapsed = 0;
        }
        is_turned_on
    }

    pub fn minutes_until_change(&self) -> u64 {
        let minute_in_cycle = self.minutes_elapsed % self.duty_interval;
        let burst_start = self.duty_interval.saturating_sub(self.duty_cycle);
        if self.is_turned_on() {
            self.duty_interval.saturating_sub(minute_in_cycle)
        } else {
            burst_start.saturating_sub(minute_in_cycle)
        }
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }

    pub fn reset(&mut self) {
        self.exhaust.off();
        self.minutes_elapsed = 0;
    }
}

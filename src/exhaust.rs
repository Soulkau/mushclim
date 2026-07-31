use core::{fmt::Write, ops::Sub};

use driverse::relay::Relay;
use embassy_time::{Duration, Instant};
use esp_hal::gpio::Output;
use heapless::String;

use crate::MushclimConfig;

pub struct ExhaustManager<'a> {
    pub exhaust: Relay<Output<'a>>,
    pub duty_cycle: Duration,
    pub duty_interval: Duration,
    cycle_start: Instant,
    last_tick: Instant,
}

impl<'a> ExhaustManager<'a> {
    pub fn new(exhaust: Relay<Output<'a>>, config: &MushclimConfig) -> Self {
        Self {
            exhaust,
            duty_cycle: config.exhaust_duty_cycle,
            duty_interval: config.exhaust_duty_interval,
            cycle_start: Instant::now(),
            last_tick: Instant::now(),
        }
    }

    pub async fn tick(&mut self, is_humidifier_on: bool) -> bool {
        let now = Instant::now();
        let dt = now - self.last_tick;
        self.last_tick = now;

        if is_humidifier_on {
            // Если увлажнитель включен - не считаем это время, "замораживаем" цикл,
            // сдвигая cycle_start вперёд на dt, чтобы elapsed_in_cycle не менялся
            self.cycle_start += dt;

            if self.exhaust.is_on() {
                defmt::info!("ExhaustManager: Paused, humidifier active.");
            }
            self.exhaust.off();
            return false;
        }

        let elapsed = now - self.cycle_start;
        //Если прошло больше либо столько же времени,как и переодичность включения, "обернуть", задать новый старт цикла.
        if elapsed >= self.duty_interval {
            let overshoot = elapsed.as_ticks() % self.duty_interval.as_ticks();
            self.cycle_start = now - Duration::from_ticks(overshoot);
        }

        //Сколько времени прошло с начала цикла
        let elapsed_in_cycle = now - self.cycle_start;
        //Отнять от интервала обдува время обдува, (например 30 - 3 = 27 (c 27 минуты включается обдув))
        let burst_start = self.duty_interval.sub(self.duty_cycle);
        //Если время с начала цилка >=старту обдува
        let should_be_on = elapsed_in_cycle >= burst_start;

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

    fn seconds_until_change(&self) -> u64 {
        let elapsed_in_cycle = Instant::now() - self.cycle_start;
        let burst_start = self.duty_interval.sub(self.duty_cycle);
        if self.is_turned_on() {
            self.duty_interval.sub(elapsed_in_cycle).as_secs()
        } else {
            burst_start.sub(elapsed_in_cycle).as_secs()
        }
    }

    pub fn format_state(&self) -> String<16> {
        let mut string = String::new();
        let _ = string.write_str("Fan: ");
        let until = self.seconds_until_change();
        if self.is_turned_on() {
            let _ = string.write_str("off");
        } else {
            let _ = string.write_str("on");
        }
        let _ = write!(string, " in {:.1}m", until as f64 / 60.0);
        string
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }

    pub fn reset(&mut self) {
        self.exhaust.off();
    }
}

use core::{fmt::Write, ops::Sub};

use embassy_time::{Duration, Instant};
use heapless::String;

pub struct CycleTimer {
    interval: Duration,
    work: Duration,
    cycle_start: Instant,
    last_tick: Instant,
}

impl CycleTimer {
    pub fn new(interval: Duration, work: Duration) -> Self {
        Self {
            interval: interval + work, //Because of how tick works, by it's design interval would be interval - work
            work,
            cycle_start: Instant::now(),
            last_tick: Instant::now(),
        }
    }

    pub fn ingore_tick(&mut self) {
        let now = Instant::now();
        let dt = now - self.last_tick;
        self.last_tick = now;
        self.cycle_start += dt;
        tracing::info!("[CycleTimer]: ignored tick for {}", &dt)
    }

    fn elapsed_wrapped(&self) -> Duration {
        let elapsed = Instant::now() - self.cycle_start;
        if elapsed >= self.interval {
            let overshoot = elapsed.as_ticks() % self.interval.as_ticks();
            Duration::from_ticks(overshoot)
        } else {
            elapsed
        }
    }

    fn burst_start(&self) -> Duration {
        self.interval.sub(self.work)
    }

    pub fn duty_started(&self) -> bool {
        self.elapsed_wrapped() >= self.burst_start()
    }

    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.last_tick = now;
        let elapsed = now - self.cycle_start;
        if elapsed >= self.interval {
            tracing::info!(
                "[CycleTimer]: cycle has ended, wrapping around {}",
                &elapsed
            );
            let overshoot = elapsed.as_ticks() % self.interval.as_ticks();
            self.cycle_start = now - Duration::from_ticks(overshoot);
        }
        tracing::info!("[CycleTimer]: ticked successfully");
        self.elapsed_wrapped() >= self.burst_start()
    }

    pub fn time_until_change(&self) -> (bool, String<4>) {
        let elapsed = self.elapsed_wrapped();
        let burst_start = self.burst_start();
        let started = elapsed >= burst_start;

        let remaining = if started {
            self.interval.sub(elapsed)
        } else {
            burst_start.sub(elapsed)
        };

        (started, self.format_time_string(remaining))
    }

    pub fn reset(&mut self) {
        self.cycle_start = Instant::now();
        self.last_tick = Instant::now();
    }

    fn format_time_string(&self, remaining: Duration) -> String<4> {
        let mut string = String::new();

        if remaining.as_secs() < 60 {
            let _ = write!(string, "{}s", remaining.as_secs());
        } else {
            let minutes = remaining.as_secs() / 60;
            let _ = write!(string, "{}m", minutes);
        }
        tracing::info!("[CycleTimer]: {} left until change", string);
        string
    }
}

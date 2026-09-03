use core::{fmt::Write, ops::Sub};

use embassy_time::{Duration, Instant};
use heapless::String;
/// Timer for creation of cycle-based tasks
///
/// # Examples
///
/// ```
/// let mut timer = CycleTimer::new(Duration::from_secs(30), Duration::from_secs(20)); // Interval: 30s, Duty: 20s
/// if timer.tick() {
///     // Start work
/// }
/// delay.time_until_next_tick().await;
/// if !timer.tick() {
///     // Work duration has passed and this will return false, stop work
/// }
/// ```
///
pub struct CycleTimer {
    /// Delay between cycles
    interval: Duration,
    /// Duration of work each cycle
    duty: Duration,
    /// Start of the current cycle
    cycle_start: Instant,
    /// Time of the last tick
    last_tick: Instant,
}

impl CycleTimer {
    pub fn new(interval: Duration, duty: Duration) -> Self {
        Self {
            interval: interval + duty, //Otherwise interval would (unironically) be interval - duty
            duty,
            cycle_start: Instant::now(),
            last_tick: Instant::now(),
        }
    }

    pub fn ingore_tick(&mut self) {
        let now = Instant::now();
        let dt = now - self.last_tick;
        self.last_tick = now;
        self.cycle_start += dt;
        tracing::debug!("[CycleTimer]: ignored tick for {}", &dt)
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
        self.interval.sub(self.duty)
    }

    pub fn duty_started(&self) -> bool {
        self.elapsed_wrapped() >= self.burst_start()
    }

    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.last_tick = now;
        let elapsed = now - self.cycle_start;
        if elapsed >= self.interval {
            tracing::debug!(
                "[CycleTimer]: cycle has ended, wrapping around {}",
                &elapsed
            );
            let overshoot = elapsed.as_ticks() % self.interval.as_ticks();
            self.cycle_start = now - Duration::from_ticks(overshoot);
        }
        tracing::debug!("[CycleTimer]: ticked successfully");
        self.elapsed_wrapped() >= self.burst_start()
    }

    pub fn time_until_change(&self) -> (bool, Duration) {
        let elapsed = self.elapsed_wrapped();
        let burst_start = self.burst_start();
        let started = elapsed >= burst_start;

        let remaining = if started {
            self.interval.sub(elapsed)
        } else {
            burst_start.sub(elapsed)
        };

        (started, remaining)
    }

    pub fn reset(&mut self) {
        self.cycle_start = Instant::now();
        self.last_tick = Instant::now();
    }
}

pub struct DueTimer {
    interval: Duration,
    next_due: Instant,
}

impl DueTimer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_due: Instant::now() + interval,
        }
    }

    pub fn due(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next_due {
            return false;
        }
        self.next_due += self.interval;
        if self.next_due <= now {
            self.next_due = now + self.interval;
        }
        true
    }
}

pub(crate) trait DurationExts {
    fn pretty_string(&self) -> String<16>;
}

impl DurationExts for Duration {
    fn pretty_string(&self) -> String<16> {
        let mut string = String::new();
        let total_ms = self.as_millis();

        let (time, unit) = if total_ms < 1_000 {
            (total_ms, "ms")
        } else if total_ms < 60_000 {
            (self.as_secs(), "s")
        } else if total_ms < 3_600_000 {
            (self.as_secs() / 60, "m")
        } else {
            (self.as_secs() / 3_600, "h")
        };

        write!(string, "{}{}", time, unit).ok();

        string
    }
}

use crate::{
    config::{MushclimConfig, MushclimConfigDto},
    measurements::Measurement,
    timer::{DueTimer, DurationExts},
};
use core::{fmt::Write, ops::RangeInclusive};
use driverse::relay::Relay;
use embassy_time::Duration;
use embedded_hal::digital::OutputPin;
use heapless::String;

#[derive(Debug, Clone)]
pub struct ExhaustConfig {
    pub co2pmm_treshold: RangeInclusive<u16>,
    pub exhaust_cooldown: Duration,
    pub exhaust_timeout: Duration,
}

impl From<&MushclimConfigDto> for ExhaustConfig {
    fn from(dto: &MushclimConfigDto) -> Self {
        Self {
            co2pmm_treshold: dto.co2ppm_lower_bound..=dto.co2ppm_upper_bound,
            exhaust_cooldown: Duration::from_secs(dto.exhaust_cooldown),
            exhaust_timeout: Duration::from_secs(dto.exhaust_timeout),
        }
    }
}

#[derive(Default)]
enum ExhaustState {
    /// Default state, meanwhile co2ppm >= upper treshold.
    #[default]
    Idle,
    /// Exhaust is running until either timer ends or co2ppm <= lower treshold.
    Runnning(DueTimer),
    /// If timeout timer hits while running, this is cooldown state, to avoid instant turning of (because timeout means that fan was not able to achieve co2ppm <= lower treshold)
    Cooldown(DueTimer),
}

impl ExhaustState {
    /// Refreshes exhaust state using measurements.
    pub fn refresh(&mut self, measurement: &Measurement, config: &ExhaustConfig) -> bool {
        let co2 = measurement.co2_ppm;
        let over = co2 >= *config.co2pmm_treshold.end();
        let under = co2 <= *config.co2pmm_treshold.start();

        tracing::debug!(tag = "exhaust", "co2={} over={} under={}", co2, over, under);

        match self {
            //If state is idle and co2 is over or above treshold run exhaust with timeout
            ExhaustState::Idle => {
                if over {
                    tracing::debug!(tag = "exhaust", "idle -> running, co2 over threshold");
                    *self = ExhaustState::Runnning(DueTimer::new(config.exhaust_timeout));
                    true
                } else {
                    tracing::debug!(tag = "exhaust", "idle, staying off");
                    false
                }
            }
            // If state is running, check if timeout has passed OR co2ppm has hit the low treshould bound
            ExhaustState::Runnning(timer) => {
                if timer.due() || under {
                    tracing::debug!(
                        tag = "exhaust",
                        "running -> cooldown (timeout_due={}, under_threshold={})",
                        timer.due(),
                        under
                    );
                    *self = ExhaustState::Cooldown(DueTimer::new(config.exhaust_cooldown));
                    false
                } else {
                    tracing::debug!(tag = "exhaust", "running, staying on");
                    true
                }
            }
            //If exhaust is on cooldown, check whether it has passed and rerefresh.
            ExhaustState::Cooldown(timer) => {
                if timer.due() {
                    tracing::debug!(tag = "exhaust", "cooldown finished -> idle, re-refreshing");
                    *self = ExhaustState::Idle;
                    self.refresh(measurement, config) //Rerefresh as cooldown has passed and co2ppm maybe over the high treshold
                } else {
                    tracing::debug!(tag = "exhaust", "cooldown, waiting");
                    false
                }
            }
        }
    }

    pub fn as_log(&self) -> String<32> {
        let mut buf = String::new();
        match self {
            ExhaustState::Idle => {
                let _ = write!(buf, "Idle");
            }
            ExhaustState::Runnning(timer) => {
                let _ = write!(
                    buf,
                    "Running (due: {})",
                    timer.time_remaining().pretty_string()
                );
            }
            ExhaustState::Cooldown(timer) => {
                let _ = write!(
                    buf,
                    "Cooldown (due: {})",
                    timer.time_remaining().pretty_string()
                );
            }
        }

        buf
    }
}

pub(crate) struct Exhaust<P: OutputPin> {
    exhaust: Relay<P>,
    exhaust_config: ExhaustConfig,
    state: ExhaustState,
}

impl<P: OutputPin> Exhaust<P> {
    pub fn new(exhaust: Relay<P>, config: &ExhaustConfig) -> Self {
        Self {
            exhaust,
            exhaust_config: config.clone(),
            state: ExhaustState::default(),
        }
    }

    pub fn tick_with_measurements(&mut self, measurements: &Measurement) -> bool {
        let should_be_on = self.state.refresh(measurements, &self.exhaust_config);
        self.switch(should_be_on)
    }

    fn switch(&mut self, on: bool) -> bool {
        let is_on = self.exhaust.is_on();
        if on && !is_on {
            self.exhaust.on().ok();
            tracing::debug!(tag = "exhaust", "Turned on");
        } else if !on && is_on {
            self.exhaust.off().ok();
            tracing::debug!(tag = "exhaust", "Turned off");
        }
        on
    }

    pub fn off(&mut self) {
        self.switch(false);
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.exhaust_config = config.exhaust.clone();
    }

    pub fn state_log(&self) -> String<32> {
        self.state.as_log()
    }
}

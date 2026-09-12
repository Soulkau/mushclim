use crate::{
    config::{MushclimConfig, MushclimConfigDto},
    measurements::Measurement,
    timer::{CycleTimer, DueTimer, DurationExts},
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
    pub safe_exhaust_duty: Duration,
    pub safe_exhaust_duty_interval: Duration,
}

impl From<&MushclimConfigDto> for ExhaustConfig {
    fn from(dto: &MushclimConfigDto) -> Self {
        Self {
            co2pmm_treshold: dto.co2ppm_lower_bound..=dto.co2ppm_upper_bound,
            exhaust_cooldown: Duration::from_secs(dto.exhaust_cooldown),
            exhaust_timeout: Duration::from_secs(dto.exhaust_timeout),
            safe_exhaust_duty: Duration::from_secs(dto.safe_exhaust_duty),
            safe_exhaust_duty_interval: Duration::from_secs(dto.safe_exhaust_duty_interval),
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

        tracing::debug!("[Exhaust]: co2={} over={} under={}", co2, over, under);

        match self {
            //If state is idle and co2 is over or above treshold run exhaust with timeout
            ExhaustState::Idle => {
                if over {
                    tracing::debug!("[Exhaust]: idle -> running, co2 over threshold");
                    *self = ExhaustState::Runnning(DueTimer::new(config.exhaust_timeout));
                    true
                } else {
                    tracing::debug!("[Exhaust]: idle, staying off");
                    false
                }
            }
            // If state is running, check if timeout has passed OR co2ppm has hit the low treshould bound
            ExhaustState::Runnning(timer) => {
                if timer.due() || under {
                    tracing::debug!(
                        "[Exhaust]: running -> cooldown (timeout_due={}, under_threshold={})",
                        timer.due(),
                        under
                    );
                    *self = ExhaustState::Cooldown(DueTimer::new(config.exhaust_cooldown));
                    false
                } else {
                    tracing::debug!("[Exhaust]: running, staying on");
                    true
                }
            }
            //If exhaust is on cooldown, check whether it has passed and rerefresh.
            ExhaustState::Cooldown(timer) => {
                if timer.due() {
                    tracing::debug!("[Exhaust]: cooldown finished -> idle, re-refreshing");
                    *self = ExhaustState::Idle;
                    self.refresh(measurement, config) //Rerefresh as cooldown has passed and co2ppm maybe over the high treshold
                } else {
                    tracing::debug!("[Exhaust]: cooldown, waiting");
                    false
                }
            }
        }
    }
}

enum Mode {
    Measurement { state: ExhaustState },
    Cycle { cycle_timer: CycleTimer },
}

pub(crate) struct ExhaustManager<P: OutputPin> {
    exhaust: Relay<P>,
    exhaust_config: ExhaustConfig,
    mode: Mode,
}

impl<P: OutputPin> ExhaustManager<P> {
    pub fn new(exhaust: Relay<P>, config: &ExhaustConfig) -> Self {
        Self {
            exhaust,
            exhaust_config: config.clone(),
            mode: Mode::Measurement {
                state: ExhaustState::default(),
            },
        }
    }

    pub fn switch_to_cycle(&mut self) {
        tracing::info!("[Exhaust]: switching to cycle mode");
        self.mode = Mode::Cycle {
            cycle_timer: CycleTimer::new(
                self.exhaust_config.safe_exhaust_duty_interval,
                self.exhaust_config.safe_exhaust_duty,
            ),
        };
        self.switch(false);
    }

    pub fn switch_to_measurement(&mut self) {
        tracing::info!("[Exhaust]: switching to measurement mode");
        self.mode = Mode::Measurement {
            state: ExhaustState::default(),
        };
        self.switch(false);
    }

    pub fn tick_with_measurements(&mut self, measurements: &Measurement) -> bool {
        let Mode::Measurement { state } = &mut self.mode else {
            tracing::warn!("[Exhaust]: tick_measurement called while in Cycle mode, ignoring");
            return self.exhaust.is_on();
        };
        let should_be_on = state.refresh(measurements, &self.exhaust_config);
        self.switch(should_be_on)
    }

    pub fn tick_cycle(&mut self) -> bool {
        let Mode::Cycle { cycle_timer } = &mut self.mode else {
            tracing::warn!("[Exhaust]: tick_cycle called while in Measurement mode, ignoring");
            return self.exhaust.is_on();
        };
        let should_be_on = cycle_timer.tick();
        self.switch(should_be_on)
    }

    fn switch(&mut self, should_be_on: bool) -> bool {
        let is_on = self.exhaust.is_on();
        if should_be_on && !is_on {
            self.exhaust.on().ok();
            tracing::debug!("[Exhaust]: Turned on");
        } else if !should_be_on && is_on {
            self.exhaust.off().ok();
            tracing::debug!("[Exhaust]: Turned off");
        }
        should_be_on
    }

    pub fn is_turned_on(&self) -> bool {
        self.exhaust.is_on()
    }
    pub fn reset(&mut self) {
        self.exhaust.off().ok();
    }
    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.exhaust_config = config.exhaust.clone();
    }

    pub fn state_log(&self) -> String<32> {
        match &self.mode {
            Mode::Measurement { .. } => {
                /* your existing display logic */
                todo!()
            }
            Mode::Cycle { cycle_timer } => {
                let (is_on_duty, until_switch) = cycle_timer.time_until_change();
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
        }
    }
}

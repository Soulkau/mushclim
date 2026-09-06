use driverse::{relay::Relay, stcc4::Stcc4};
use embassy_futures::select::{Either, select};
use embassy_time::{Ticker, Timer};
use ivy::{
    mqtt::{MqttHandle, Subscription},
    storage::{StorageKey, StorageModule},
};

use crate::{
    MushclimConfig, MushclimConfigDto, MushclimPlatform, MushclimStats,
    exhaust::ExhaustManager,
    humidifier::Humidifier,
    measurements::{Measurement, MeasurementError, MeasurementManager},
    metrics::MetricManager,
    timer::CycleTimer,
};

const CONF_KEY: StorageKey = StorageKey::new(101);

pub type MushclimMqttHandle = MqttHandle<512>;

macro_rules! create_relay {
    ($pin:expr) => {
        Relay::new($pin, driverse::relay::ActiveLevel::Low, false).unwrap()
    };
}

pub struct MushclimPins<P: MushclimPlatform> {
    pub exhaust: P::ExhaustPin,
    pub light: P::LightPin,
    pub disco: P::DiscoPin,
    pub humidifier: P::HumidifierPin,
    pub sensor: P::Sensor,
    pub delay: P::Delay,
}

pub struct MushclimApp<P: MushclimPlatform> {
    measurement_manager: MeasurementManager<P::Sensor, P::Delay>,
    exhaust: ExhaustManager<P::ExhaustPin>,
    config: MushclimConfig,
    lights: Relay<P::LightPin>,
    humidifier: Humidifier<P::HumidifierPin>,
    disco: Relay<P::DiscoPin>,
    metrics: MetricManager,
    config_sub: Subscription<MushclimConfigDto>,
    mqtt_handle: MushclimMqttHandle,
    storage: StorageModule<P::FlashStorage>,
}

impl<P: MushclimPlatform> MushclimApp<P> {
    pub fn new(
        pins: MushclimPins<P>,
        config: MushclimConfig,
        config_sub: Subscription<MushclimConfigDto>,
        mqtt_handle: MushclimMqttHandle,
        storage: StorageModule<P::FlashStorage>,
    ) -> Self {
        Self {
            measurement_manager: MeasurementManager::new(Stcc4::new(pins.sensor, pins.delay)),
            exhaust: ExhaustManager::new(create_relay!(pins.exhaust), &config),
            lights: create_relay!(pins.light),
            humidifier: Humidifier::new(create_relay!(pins.humidifier), &config),
            disco: create_relay!(pins.disco),
            metrics: MetricManager::new(&config),
            config,
            config_sub,
            mqtt_handle,
            storage,
        }
    }

    pub async fn run(&mut self) -> ! {
        self.lights.on().ok();
        self.disco.on().ok();

        tracing::info!("[MushclimApp] Starting sensor calibration");
        self.measurement_manager.init().await;

        self.exhaust.timer.ingore_tick();
        let mut ticker = Ticker::every(self.config.loop_delay);

        loop {
            self.tick_step().await;

            // Wait for either the next timer tick or an incoming config update
            match select(ticker.next(), self.config_sub.next()).await {
                Either::First(_) => {
                    // Timer expired normally, loop around for next tick
                }
                Either::Second(config_dto) => {
                    tracing::info!("[MushclimApp] Config update received");
                    self.hard_config_update(config_dto.as_local()).await;
                    self.storage.set(CONF_KEY, &config_dto).await;
                    ticker = Ticker::every(self.config.loop_delay);
                }
            }
        }
    }

    async fn hard_config_update(&mut self, config: MushclimConfig) {
        self.config = config;
        self.exhaust.force_config(&self.config);
        self.metrics.force_config(&self.config);
        self.humidifier.update_config(&self.config);
        tracing::info!("[MushclimApp] Hard config update performed");
    }

    async fn tick_step(&mut self) {
        self.exhaust.tick().await;

        match self.acquire_safe_measurement().await {
            Some(measurements) => {
                self.metrics.feed(measurements, &self.mqtt_handle).await;

                self.humidifier
                    .tick(&measurements, self.exhaust.is_turned_on());
                self.log_app_state(measurements).await;
            }
            None => {
                tracing::error!(
                    "[MushclimApp] CRITICAL: Sensor totally failed or reading is permanently erratic"
                );
                self.enter_safe_mode(1).await;
            }
        }
    }

    async fn acquire_safe_measurement(&mut self) -> Option<Measurement> {
        // Attempt up to 5 times to get a stable, validated reading
        for attempt in 1..=self.config.retry_count {
            match self.measurement_manager.measure(&self.config).await {
                Ok(stats) => {
                    // It passed hardware check AND history check!
                    return Some(stats);
                }
                Err(MeasurementError::Sensor) => {
                    // "Замерить датчик -> err -> попробовать еще раз (5 попыток)"
                    tracing::warn!("Hardware read error on attempt {}. Retrying...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::Crc) => {
                    tracing::warn!("Sensor crc mismatch {}. Retrying shortly.", attempt);
                    Timer::after_secs(2).await;
                }
            }
        }
        None
    }

    async fn enter_safe_mode(&mut self, code: u32) -> ! {
        tracing::error!("Entering safemode, errcode: {}", code);

        self.humidifier.turn_off();
        self.exhaust.reset();

        let mut humidity_timer = CycleTimer::new(
            self.config.safe_humidity_duty_interval,
            self.config.safe_humidity_duty,
        );

        loop {
            self.exhaust.tick().await;

            let humidifier_on = if self.exhaust.is_turned_on() && humidity_timer.duty_started() {
                humidity_timer.ingore_tick();
                if self.humidifier.is_on() {
                    tracing::info!("[SafeMode]: humidifier is on along exhaust, turning off");
                    self.humidifier.turn_off();
                }
                false
            } else {
                humidity_timer.tick()
            };

            if humidifier_on != self.humidifier.is_on() {
                if humidifier_on {
                    tracing::info!("[SafeMode]: Humidifer is on");
                    self.humidifier.turn_on();
                } else {
                    tracing::info!("[SafeMode]: Humidifer is off");
                    self.humidifier.turn_off();
                }
            }

            Timer::after(self.config.loop_delay).await;
        }
    }

    async fn log_app_state(&self, measurements: Measurement) {
        let stats = MushclimStats {
            temperature: measurements.temperature_c as i16,
            humidity: measurements.humidity_pct as u16,
            exhaust_on: self.exhaust.is_turned_on(),
            humidifier_on: self.humidifier.is_on(),
        };
        tracing::info!(
            "[MushclimApp] Temp: {}°C, Humidity: {}% Humidifier: {}, Exhaust: {}",
            stats.temperature,
            stats.humidity,
            stats.humidifier_on,
            self.exhaust.state_log()
        );
        self.mqtt_handle.publish("mushclim/stats", stats).await;
    }
}

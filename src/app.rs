use driverse::{relay::Relay, stcc4::Stcc4};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, Timer};
use ivy::{
    flash::{FlashStorage, StorageKey},
    mqtt::{SizedMqttHandle, Subscription},
};

use crate::{
    MushclimPlatform, MushclimStatus,
    config::{MushclimConfig, MushclimConfigDto},
    exhaust::Exhaust,
    humidifier::Humidifier,
    measurements::{Measurement, MeasurementError, MeasurementManager},
};

const CONF_KEY: StorageKey = StorageKey::new(101);

pub const MUSHCLIM_MQTT_PAYLOAD: usize = 512;

pub type MushclimMqttHandle = SizedMqttHandle<MUSHCLIM_MQTT_PAYLOAD>;

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
    exhaust: Exhaust<P::ExhaustPin>,
    config: MushclimConfig,
    lights: Relay<P::LightPin>,
    humidifier: Humidifier<P::HumidifierPin>,
    disco: Relay<P::DiscoPin>,
    config_sub: Subscription<MushclimConfigDto>,
    mqtt_handle: MushclimMqttHandle,
    storage: FlashStorage<P::NvsStorage>,
}

impl<P: MushclimPlatform> MushclimApp<P> {
    pub async fn init(
        pins: MushclimPins<P>,
        storage: FlashStorage<P::NvsStorage>,
        mqtt_handle: MushclimMqttHandle,
        config_sub: Subscription<MushclimConfigDto>,
    ) -> Self {
        let config = storage
            .get::<MushclimConfigDto>(CONF_KEY)
            .await
            .unwrap_or(MushclimConfigDto::default())
            .as_local();

        Self {
            measurement_manager: MeasurementManager::new(Stcc4::new(pins.sensor, pins.delay)),
            exhaust: Exhaust::new(create_relay!(pins.exhaust), &config.exhaust),
            lights: create_relay!(pins.light),
            humidifier: Humidifier::new(create_relay!(pins.humidifier), &config.humidifier),
            disco: create_relay!(pins.disco),
            config,
            config_sub,
            mqtt_handle,
            storage,
        }
    }

    pub async fn run(&mut self) -> ! {
        self.lights.on().ok();
        self.disco.on().ok();

        tracing::info!(tag = "app", "starting sensor calibration");
        self.measurement_manager.init().await;
        tracing::info!(tag = "app", "finished sensor calibration");
        let mut ticker = Ticker::every(self.config.loop_delay);

        loop {
            self.tick_step().await;

            // Wait for either the next timer tick or an incoming config update
            match select(ticker.next(), self.config_sub.next()).await {
                Either::First(_) => {
                    // Timer expired normally, loop around for next tick
                }
                Either::Second(config_dto) => {
                    tracing::info!(tag = "app", "config update received");
                    self.hard_config_update(config_dto.as_local()).await;
                    self.storage.set(CONF_KEY, &config_dto).await;
                    ticker = Ticker::every(self.config.loop_delay);
                    tracing::info!(tag = "app", "config update completed");
                }
            }
        }
    }
    /// Update configs for everyone
    async fn hard_config_update(&mut self, config: MushclimConfig) {
        self.config = config;
        self.exhaust.force_config(&self.config);
        self.humidifier.force_config(&self.config);
    }
    ///Tick app single time.
    async fn tick_step(&mut self) {
        match self.acquire_measurement().await {
            Some(measurements) => {
                self.exhaust.tick_with_measurements(&measurements);
                self.humidifier
                    .tick(&measurements, self.exhaust.is_turned_on());
                self.log_app_state(measurements).await;
            }
            None => {
                tracing::warn!(tag = "app", "failed to get measurements");
                self.sensor_failure_mode().await;
            }
        }
        tracing::info!(tag = "ping", "ok");
    }

    /// Attempts to get measurement, will retry `config.retry_count` times, before giving up.
    async fn acquire_measurement(&mut self) -> Option<Measurement> {
        let retry_delay = Duration::from_secs(3);
        for attempt in 1..=self.config.retry_count {
            match self.measurement_manager.measure().await {
                Ok(stats) => {
                    return Some(stats);
                }
                Err(MeasurementError::Sensor) => {
                    tracing::warn!(
                        tag = "app",
                        "hardware read error on attempt {}. Retrying...",
                        attempt
                    );
                    Timer::after(retry_delay).await;
                }
                Err(MeasurementError::Crc) => {
                    tracing::warn!(
                        tag = "app",
                        "sensor crc mismatch {}. Retrying shortly.",
                        attempt
                    );
                    Timer::after(retry_delay).await;
                }
            }
        }
        None
    }

    async fn sensor_failure_mode(&mut self) {
        self.exhaust.off();
        self.humidifier.off();
        tracing::error!(tag = "fatal", "sensor failed");
        loop {
            Timer::after(self.config.loop_delay).await;
            if self.acquire_measurement().await.is_some() {
                tracing::info!(tag = "app", "sensor recovered");
                break;
            }
        }
    }

    async fn log_app_state(&self, measurements: Measurement) {
        let stats = MushclimStatus {
            temperature: measurements.temperature_c as i16,
            humidity: measurements.humidity_pct as u16,
            co2ppm: measurements.co2_ppm,
            exhaust_on: self.exhaust.is_turned_on(),
            humidifier_on: self.humidifier.is_on(),
        };
        tracing::debug!(
            tag = "app",
            "Temp: {}°C, Humidity: {}% Humidifier: {}, Exhaust: {}",
            stats.temperature,
            stats.humidity,
            stats.humidifier_on,
            self.exhaust.state_log()
        );
        self.mqtt_handle
            .publish("mushclim/status", stats)
            .await
            .ok();
    }
}

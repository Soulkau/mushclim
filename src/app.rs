use driverse::relay::Relay;
use embassy_futures::select::{Either, select};
use embassy_time::{Ticker, Timer};
use ivy::{
    mqtt::{MqttHandle, Subscription},
    storage::{StorageKey, StorageModule},
};

use crate::{
    MushclimConfig, MushclimConfigDto, MushclimPlatform,
    exhaust::ExhaustManager,
    humidifier::Humidifier,
    measurements::{MeasurementError, MeasurementManager, MeasurementProvider, Measurements},
    metrics::MetricManager,
    timer::CycleTimer,
};

const CONF_KEY: StorageKey = StorageKey::new(101);

struct MushclimApp<P: MushclimPlatform> {
    measurement_manager: MeasurementManager<P::Measurement>,
    exhaust: ExhaustManager<P::ExhaustPin>,
    config: MushclimConfig,
    lights: Relay<P::LightPin>,
    humidifier: Humidifier<P::HumidifierPin>,
    disco: Relay<P::DiscoPin>,
    metrics: MetricManager,
    config_sub: Subscription<MushclimConfigDto>,
    mqtt_handle: MqttHandle<312>,
    storage: StorageModule<P::FlashStorage>,
}

impl<P: MushclimPlatform> MushclimApp<P> {
    pub async fn run(&mut self) -> ! {
        self.lights.on();
        self.disco.on();

        tracing::info!("[MushclimApp] Starting sensor calibration");
        self.display_calibrating().await;

        let Ok(_) = self.measurement_manager.calibrate(&self.config).await else {
            tracing::error!("[MushclimApp] Failed to calibrate measurements");
            self.enter_safe_mode(4).await
        };

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

                self.display_measurements(measurements).await;
                self.humidifier
                    .tick(&measurements, self.exhaust.is_turned_on());
                self.log_current_measurements(measurements, self.humidifier.is_on())
                    .await;
                self.send_stats(
                    measurements,
                    self.exhaust.is_turned_on(),
                    self.humidifier.is_on(),
                )
                .await;
            }
            None => {
                tracing::error!(
                    "[MushclimApp] CRITICAL: Sensor totally failed or reading is permanently erratic"
                );
                self.enter_safe_mode(1).await;
            }
        }
    }

    async fn acquire_safe_measurement(&mut self) -> Option<Measurements> {
        // Attempt up to 5 times to get a stable, validated reading
        for attempt in 1..=self.config.retry_count {
            match self.measurement_manager.get_measurements(&self.config) {
                Ok(stats) => {
                    // It passed hardware check AND history check!
                    return Some(stats);
                }
                Err(MeasurementError::SensorDriver(_e)) => {
                    // "Замерить датчик -> err -> попробовать еще раз (5 попыток)"
                    tracing::warn!("Hardware read error on attempt {}. Retrying...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::ErraticReading) => {
                    // "сверить новое измерение с историей -> err -> повторить замер и сравнить (5 раз)"
                    tracing::warn!("Spike detected on attempt {}. Re-measuring...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::NotCalibrated) => {
                    self.enter_safe_mode(3).await;
                }
            }
        }
        None
    }

    async fn enter_safe_mode(&mut self, error_code: u32) -> ! {
        tracing::error!("Entering safe mode");

        self.humidifier.turn_off();
        self.exhaust.reset();
        self.display_safe_mode(error_code).await;

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

    pub async fn display_calibrating(&mut self) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(0, 255, 0).await;
            let _ = lcd.write_str_no("Calibrating").await;
        }
    }

    pub async fn display_measurements(&mut self, stats: Measurements) {
        if let Some(lcd) = self.lcd.as_mut() {
            let mut line: String<32> = String::new();
            let _ = write!(line, "T:{:.1}C H:{:.0}%", stats.temperature, stats.humidity);
            let _ = lcd
                .write_str(
                    0,
                    &line,
                    &WriteSettings::new().align(TextAlign::Center).clear(false),
                )
                .await;
        }
    }

    pub async fn display_fan_state(&mut self, line: String<16>) {
        tracing::info!("[MushclimApp] fan state: {}", line);
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.clear_row(1).await;
            let _ = lcd
                .write_str(
                    1,
                    &line,
                    &WriteSettings::new().align(TextAlign::Center).clear(false),
                )
                .await;
        }
    }

    /// Flip the LCD into a visual "safe mode" indicator: red backlight + message.
    pub async fn display_safe_mode(&mut self, error_code: u32) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(125, 0, 0).await;

            let msg = match error_code {
                1 => "Err01 Sensor",
                2 => "Err02 Erratic",
                3 => "Err03 NoCalib",
                4 => "Err04 CalibFail",
                5 => "Err05 Exhaust",
                _ => "Err00 Unknown",
            };

            let mut line: String<32> = String::new();
            let _ = write!(line, "{}", msg);
            let _ = lcd.write_str_no(&line).await;
        }
    }

    async fn send_stats(&self, measurements: Measurements, exhaust_on: bool, humidifier_on: bool) {
        let stats = MushclimStats {
            temperature: measurements.temperature,
            humidity: measurements.humidity,
            exhaust_on,
            humidifier_on,
        };

        self.mqtt_handle.publish("mushclim/stats", stats).await;
    }

    /// Helper to grab measurements and dump them to the logger
    async fn log_current_measurements(&mut self, stats: Measurements, humidifier_on: bool) {
        tracing::info!(
            "[MushclimApp] Temp: {}°C, Humidity: {}% Humidifier: {}",
            stats.temperature,
            stats.humidity,
            humidifier_on
        );
    }
}

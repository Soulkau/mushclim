use core::{convert::Infallible, fmt::Debug, fmt::Write};

use driverse::am2301::{self, Am2301};
use embassy_time::{Delay, Timer};
use esp_hal::gpio::Flex;
use heapless::String;

use crate::{
    MushclimConfig,
    lcd::{Lcd, TextAlign, WriteSettings},
};

#[derive(thiserror::Error, Debug)]
pub enum MeasurementError<E> {
    /// The sensor hardware failed (IO, Timeout, CRC, etc.)
    SensorDriver(#[from] E),
    /// The reading jumped way too fast (failed the delta filter)
    ErraticReading,
    /// Not calibrated
    NotCalibrated,
}

pub enum HistoryState {
    Calibrating(),
}

struct MeasurementHistory {
    data: [Option<Measurements>; 5],
    write_index: usize,
}

impl MeasurementHistory {
    pub fn new() -> Self {
        Self {
            data: [None; 5],
            write_index: 0,
        }
    }

    pub fn push(&mut self, measurements: Measurements) {
        self.data[self.write_index] = Some(measurements);
        self.write_index = (self.write_index + 1) % 5;
    }

    pub fn pull(&mut self) -> Option<Measurements> {
        let mut res = Measurements {
            humidity: 0,
            temperature: 0,
        };
        let mut count = 0;
        for opt in self.data.iter() {
            if let Some(measurements) = opt {
                res.humidity += measurements.humidity;
                res.temperature += measurements.temperature;
                count += 1;
            }
        }

        if count <= 0 {
            return None;
        }

        res.humidity /= count as u16;
        res.temperature /= count as i16;

        Some(res)
    }
}

pub struct MeasurementManager<P: MeasurementProvider> {
    history: MeasurementHistory,
    primary_provider: P,
}

impl<P: MeasurementProvider> MeasurementManager<P> {
    pub fn new(primary_provider: P) -> Self {
        Self {
            history: MeasurementHistory::new(),
            primary_provider,
        }
    }

    pub async fn calibrate(
        &mut self,
        config: &MushclimConfig,
        mut lcd: Option<&mut Lcd>,
    ) -> Result<(), P::Error> {
        // --- nested helpers ---

        async fn write_header(lcd: &mut Lcd) {
            let _ = lcd
                .write_str(
                    0,
                    "Calibrating",
                    &WriteSettings::new().align(TextAlign::Center),
                )
                .await;
        }

        async fn write_status_line(lcd: &mut Lcd, done: usize, total: usize, m: &Measurements) {
            let mut line: String<32> = String::new();
            let _ = write!(
                line,
                "{}/{} T{:.0}C H{:.0}%",
                done, total, m.temperature, m.humidity
            );
            let _ = lcd
                .write_str(
                    1,
                    &line,
                    &WriteSettings::new().align(TextAlign::Center).clear(false),
                )
                .await;
        }

        // --- main logic ---

        let mut successful_samples = 0;
        let mut consecutive_failures = 0;

        if let Some(l) = lcd.as_mut() {
            write_header(l).await;
        }

        while successful_samples < config.calibration_samples {
            match self.get_raw_measurements() {
                Ok(m) => {
                    self.history.push(m);
                    successful_samples += 1;
                    consecutive_failures = 0;
                    log::info!(
                        "Calibration sample {}/10 stored successfully. Temp: {}, Hum: {}",
                        successful_samples,
                        m.temperature,
                        m.humidity
                    );

                    if let Some(l) = lcd.as_mut() {
                        write_status_line(l, successful_samples, config.calibration_samples, &m)
                            .await;
                    }

                    Timer::after_secs(2).await;
                }
                Err(e) => {
                    consecutive_failures += 1;
                    log::warn!(
                        "Hardware glitched during calibration (Failure {}/5): {:?}",
                        consecutive_failures,
                        e
                    );
                    if consecutive_failures >= 5 {
                        log::error!("CRITICAL: Sensor failed 5 times in a row during calibration!");
                        return Err(e);
                    }
                    Timer::after_secs(2).await;
                }
            }
        }
        Ok(())
    }

    pub fn get_measurements(
        &mut self,
        config: &MushclimConfig,
    ) -> Result<Measurements, MeasurementError<<P as MeasurementProvider>::Error>> {
        let measurements = self.primary_provider.get_measurements()?;
        let median = self.history.pull().ok_or(MeasurementError::NotCalibrated)?;
        if !measurements.compare(&median, config) {
            return Err(MeasurementError::ErraticReading);
        }
        Ok(measurements)
    }

    fn get_raw_measurements(&mut self) -> Result<Measurements, P::Error> {
        self.primary_provider.get_measurements()
    }
}

pub trait MeasurementProvider {
    type Error: Debug;
    fn get_measurements(&mut self) -> Result<Measurements, Self::Error>;
}

#[derive(Debug, Clone, Copy)]
pub struct Measurements {
    pub temperature: i16,
    pub humidity: u16,
}

impl Measurements {
    /// Checks if a new reading deviates too much from a baseline (average/median).
    /// Returns `true` if the reading is stable, and `false` if it jumps too wildly.
    pub fn compare(&self, baseline: &Measurements, config: &MushclimConfig) -> bool {
        let temp_delta = self.temperature.abs_diff(baseline.temperature);
        let hum_delta = self.humidity.abs_diff(baseline.humidity);

        temp_delta <= config.max_temp_delta && hum_delta <= config.max_humidity_delta
    }
}

impl<'a> MeasurementProvider for Am2301<'a, Flex<'static>> {
    type Error = am2301::Error<Infallible>;

    fn get_measurements(&mut self) -> Result<Measurements, Self::Error> {
        let (humidity, temperature) = self.measure(&mut Delay)?;
        Ok(Measurements {
            humidity: humidity as u16,
            temperature: temperature as i16,
        })
    }
}
/*
pub struct MockSensorProvider {
    step: usize,
    // We store a list of values to feed to the app on consecutive successful loops
    simulated_readings: [(i16, u16); 15],
}

impl MockSensorProvider {
    pub fn new() -> Self {
        Self {
            step: 0,
            simulated_readings: [
                // --- Calibration Phase (10 readings) ---
                (22, 84),
                (22, 84),
                (22, 85),
                (22, 85),
                (22, 84),
                (22, 84),
                (22, 85),
                (22, 85),
                (22, 84),
                (22, 84),
                // --- Main Loop Control Phase ---
                (22, 78), // 11. Drops BELOW low bound (80%) -> Humidifier should turn ON
                (22, 83), // 12. Rising, but inside comfort range (80-88) -> Should STAY ON
                (22, 90), // 13. Exceeds comfort bound (88%) -> Should turn OFF
                (22, 85), // 14. Falling, but inside comfort range -> Should STAY OFF
                (22, 75), // 15. Drops below limit again -> Should turn ON
            ],
        }
    }
}

impl MeasurementProvider for MockSensorProvider {
    type Error = core::convert::Infallible; // Mocks don't fail!

    fn get_measurements(&mut self) -> Result<Measurements, Self::Error> {
        // Grab the reading for the current step, or just loop the last reading if we run out
        let index = core::cmp::min(self.step, self.simulated_readings.len() - 1);
        let (temp, hum) = self.simulated_readings[index];

        self.step += 1;

        Ok(Measurements {
            temperature: temp,
            humidity: hum,
        })
    }
}
*/

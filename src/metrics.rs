use crate::{
    METRIC_SNAPSHOT_AMOUNT, app::MushclimMqttHandle, config::MushclimConfig,
    measurements::Measurement, timer::DueTimer,
};
use embassy_time::Instant;
use heapless::HistoryBuf;
use serde::{Deserialize, Serialize};

const MINIMUM_METRIC_AGE_SECONDS: u64 = 360;

/// Immutable, calculated from `UnfinishedStat`
#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct Stat {
    pub min: f32,
    pub max: f32,
    pub avg: f32,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct Metric {
    pub uptime_secs: u64, // when this period *ended*, relative to boot
    pub temp: Stat,
    pub humidity: Stat,
    pub co2: Stat,
    pub samples: u32,
}
/// Structure that accamulates values, and calculates finished `Stat`
#[derive(Default)]
struct UnfinishedStat {
    min: f32,
    max: f32,
    sum: f32,
}

impl UnfinishedStat {
    fn update(&mut self, value: f32) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
    }

    fn finish(&self, samples: u32) -> Stat {
        Stat {
            min: self.min,
            max: self.max,
            avg: self.sum / samples as f32,
        }
    }
}
/// Metric that has to be finished, bakes into immutable `Metric` after finish
struct UnfinishedMetric {
    started_at: Instant,
    temp_stat: UnfinishedStat,
    humidity_stat: UnfinishedStat,
    co2_stat: UnfinishedStat,
    samples: u32,
}

impl UnfinishedMetric {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            humidity_stat: Default::default(),
            temp_stat: Default::default(),
            co2_stat: Default::default(),
            samples: 0,
        }
    }
    /// Updates all stat by measurments
    fn record(&mut self, measurement: Measurement) {
        self.temp_stat.update(measurement.temperature_c);
        self.humidity_stat.update(measurement.humidity_pct);
        self.co2_stat.update(measurement.co2_ppm as f32);
        self.samples += 1;
    }
    /// Creates finalized metric
    fn finish(&self) -> Metric {
        Metric {
            uptime_secs: Instant::now().as_secs(),
            temp: self.temp_stat.finish(self.samples),
            humidity: self.humidity_stat.finish(self.samples),
            co2: self.co2_stat.finish(self.samples),
            samples: self.samples,
        }
    }
}

///Manages metric updates and sends them via mqtt
///Has cache for last 10 metrics.
pub(crate) struct MetricManager {
    unfinished: UnfinishedMetric,
    cache: HistoryBuf<Metric, METRIC_SNAPSHOT_AMOUNT>,
    timer: DueTimer,
}

impl MetricManager {
    pub fn new(config: &MushclimConfig) -> Self {
        Self {
            unfinished: UnfinishedMetric::new(),
            cache: HistoryBuf::new(),
            timer: DueTimer::new(config.metric_send_period),
        }
    }
    /// Feeds new measurements to currently unfinished metric.
    /// Sends metrics via mqtt every `config.metric_send_period`
    pub async fn feed(&mut self, measurements: Measurement, handle: &MushclimMqttHandle) {
        self.unfinished.record(measurements);

        if !self.timer.due() {
            return;
        }

        //In case time somehow runs out before metric actually has 'usefull' information in it
        if self.unfinished.started_at.elapsed().as_secs() < MINIMUM_METRIC_AGE_SECONDS {
            tracing::warn!("[MetricManager] Metric is too new to send it, reseting it");
            self.reset_metric();
            return;
        }

        let finished = self.unfinished.finish();
        if let Err(e) = handle.publish("mushclim/metrics", finished).await {
            tracing::error!("[MetricManager] Failed to publish metrics, {}", e);
        };
        self.cache.write(finished);
        self.reset_metric();
    }

    fn reset_metric(&mut self) {
        self.unfinished = UnfinishedMetric::new();
    }

    /// Updates config, resets unfinished metric
    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.timer = DueTimer::new(config.metric_send_period);
        self.reset_metric();
    }
}

use crate::{METRIC_SNAPSHOT_AMOUNT, MushclimConfig, measurements::Measurements, timer::DueTimer};
use core::fmt::Write;
use embassy_time::Instant;
use heapless::{HistoryBuf, String};
use ivy::mqtt::MqttHandle;
use serde::{Deserialize, Serialize};
use talky::types::logs::LOG_SIZE;
#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Metric {
    pub uptime_secs: u64, // when this period *ended*, relative to boot
    pub temp_min: f32,
    pub temp_max: f32,
    pub temp_avg: f32,
    pub humidity_min: f32,
    pub humidity_max: f32,
    pub humidity_avg: f32,
    pub samples: u32,
}

struct UnfinishedMetric {
    started_at: Instant,
    temp_min: f32,
    temp_max: f32,
    temp_sum: f32,
    humidity_min: f32,
    humidity_max: f32,
    humidity_sum: f32,
    samples: u32,
}

impl UnfinishedMetric {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            temp_min: f32::MAX,
            temp_max: f32::MIN,
            temp_sum: 0.0,
            humidity_min: f32::MAX,
            humidity_max: f32::MIN,
            humidity_sum: 0.0,
            samples: 0,
        }
    }

    fn record(&mut self, temp: f32, humidity: f32) {
        self.temp_min = self.temp_min.min(temp);
        self.temp_max = self.temp_max.max(temp);
        self.temp_sum += temp;
        self.humidity_min = self.humidity_min.min(humidity);
        self.humidity_max = self.humidity_max.max(humidity);
        self.humidity_sum += humidity;
        self.samples += 1;
    }

    fn finish(&self) -> Metric {
        let n = self.samples.max(1) as f32;
        Metric {
            uptime_secs: Instant::now().as_secs(),
            temp_min: self.temp_min,
            temp_max: self.temp_max,
            temp_avg: self.temp_sum / n,
            humidity_min: self.humidity_min,
            humidity_max: self.humidity_max,
            humidity_avg: self.humidity_sum / n,
            samples: self.samples,
        }
    }
}

pub struct MetricManager {
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

    pub async fn feed(&mut self, measurements: Measurements, handle: &MqttHandle<312>) {
        self.unfinished.record(
            measurements.temperature as f32,
            measurements.humidity as f32,
        );

        if !self.timer.due() {
            return;
        }

        if self.unfinished.started_at.elapsed().as_secs() >= 1 {
            let finished = self.unfinished.finish();
            let _ = handle.publish("mushclim/metrics", finished).await;
            tracing::error!("{}", format_metric(&finished));
            self.cache.write(finished);
            self.unfinished = UnfinishedMetric::new();
        }
    }

    /// Most recent N snapshots, newest first.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &Metric> {
        self.cache.oldest_ordered().rev().take(n)
    }

    pub fn all(&self) -> impl Iterator<Item = &Metric> {
        self.cache.oldest_ordered()
    }

    pub fn force_config(&mut self, config: &MushclimConfig) {
        self.timer = DueTimer::new(config.metric_send_period);
    }
}

fn format_metric(metric: &Metric) -> String<LOG_SIZE> {
    let mut s = String::new();
    let _ = write!(
        s,
        "🍄 Hourly metrics (uptime {}m)\n\
             temp min: {:.1}  avg: {:.1}  max: {:.1}\n\
             humid min: {:.1}  avg: {:.1}  max: {:.1}\n\
             samples: {}",
        metric.uptime_secs / 60,
        metric.temp_min,
        metric.temp_avg,
        metric.temp_max,
        metric.humidity_min,
        metric.humidity_avg,
        metric.humidity_max,
        metric.samples,
    );
    s
}

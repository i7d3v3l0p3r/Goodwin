use std::{
    sync::atomic::{AtomicU32, Ordering::Relaxed},
    time::Duration,
};

use serde::Serialize;

const LATENCY_BOUNDS_US: [u32; 7] = [1_000, 5_000, 10_000, 20_000, 40_000, 80_000, 160_000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_US.len() + 1;
static DEVICE_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);
static PLUGIN_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);
static PROVIDER_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);
static TIMER_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);
static WAKE: Latency = Latency::new();
static WAKE_FAILURES: AtomicU32 = AtomicU32::new(0);
static CAPTURE_SKIPPED_FRAMES: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "espidf")]
static WAKE_CAPTURE_SAMPLES: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "espidf")]
static WAKE_MAX_LAG_SAMPLES: AtomicU32 = AtomicU32::new(0);
static VOICE_CANCELLATIONS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "espidf")]
static FLASH: Latency = Latency::new();
#[cfg(target_os = "espidf")]
static FLASH_FAILURES: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
pub(crate) enum Queue {
    #[cfg(target_os = "espidf")]
    Device,
    #[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
    Plugin,
    #[cfg(target_os = "espidf")]
    Provider,
    #[cfg(not(target_os = "espidf"))]
    Timer,
}

impl Queue {
    pub(crate) fn full(self) {
        let counter = match self {
            #[cfg(target_os = "espidf")]
            Self::Device => &DEVICE_QUEUE_FULL,
            #[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
            Self::Plugin => &PLUGIN_QUEUE_FULL,
            #[cfg(target_os = "espidf")]
            Self::Provider => &PROVIDER_QUEUE_FULL,
            #[cfg(not(target_os = "espidf"))]
            Self::Timer => &TIMER_QUEUE_FULL,
        };
        add(counter, 1);
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn send<T>(
        self,
        sender: &std::sync::mpsc::SyncSender<T>,
        value: T,
    ) -> Result<(), std::sync::mpsc::TrySendError<T>> {
        sender.try_send(value).inspect_err(|error| {
            if matches!(error, std::sync::mpsc::TrySendError::Full(_)) {
                self.full();
            }
        })
    }
}

fn add(counter: &AtomicU32, value: u32) {
    let mut count = counter.load(Relaxed);
    while count != u32::MAX {
        match counter.compare_exchange_weak(count, count.saturating_add(value), Relaxed, Relaxed) {
            Ok(_) => return,
            Err(current) => count = current,
        }
    }
}

#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
pub(crate) fn wake_frame(elapsed: Duration, failed: bool) {
    WAKE.observe(elapsed);
    if failed {
        add(&WAKE_FAILURES, 1);
    }
}

#[cfg(all(target_os = "espidf", feature = "wake-tflite"))]
pub(crate) fn capture_skipped(frames: usize) {
    add(
        &CAPTURE_SKIPPED_FRAMES,
        u32::try_from(frames).unwrap_or(u32::MAX),
    );
}

#[cfg(all(target_os = "espidf", feature = "wake-tflite"))]
pub(crate) fn wake_capture(samples: usize) {
    WAKE_CAPTURE_SAMPLES.fetch_add(samples as u32, Relaxed);
}

#[cfg(all(target_os = "espidf", feature = "wake-tflite"))]
pub(crate) fn wake_lag(samples: usize) {
    WAKE_MAX_LAG_SAMPLES.fetch_max(samples as u32, Relaxed);
}

pub(crate) fn voice_cancelled() {
    add(&VOICE_CANCELLATIONS, 1);
}

#[cfg(target_os = "espidf")]
pub(crate) fn flash_write<T, E>(operation: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let start = std::time::Instant::now();
    let result = operation();
    FLASH.observe(start.elapsed());
    if result.is_err() {
        add(&FLASH_FAILURES, 1);
    }
    result
}

struct Latency {
    buckets: [AtomicU32; LATENCY_BUCKETS],
    max_us: AtomicU32,
}

impl Latency {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU32::new(0) }; LATENCY_BUCKETS],
            max_us: AtomicU32::new(0),
        }
    }

    fn observe(&self, elapsed: Duration) {
        let elapsed_us = u32::try_from(elapsed.as_micros()).unwrap_or(u32::MAX);
        self.max_us.fetch_max(elapsed_us, Relaxed);
        let index = LATENCY_BOUNDS_US.partition_point(|&bound| bound < elapsed_us);
        add(&self.buckets[index], 1);
    }

    fn snapshot(&self) -> Timing {
        let buckets = self
            .buckets
            .each_ref()
            .map(|bucket| u64::from(bucket.load(Relaxed)));
        let count = buckets.iter().sum::<u64>();
        let max_us = self.max_us.load(Relaxed);
        let percentile = |percent: u64| {
            let rank = (count * percent).div_ceil(100);
            let mut seen = 0;
            let bucket = buckets.iter().position(|count| {
                seen += count;
                seen >= rank
            });
            bucket.map_or(max_us, |bucket| {
                LATENCY_BOUNDS_US
                    .get(bucket)
                    .copied()
                    .unwrap_or(max_us)
                    .min(max_us)
            })
        };
        Timing {
            count,
            p50_upper_us: (count > 0).then(|| percentile(50)),
            p95_upper_us: (count > 0).then(|| percentile(95)),
            max_us: (count > 0).then_some(max_us),
        }
    }
}

#[derive(Serialize)]
struct Timing {
    count: u64,
    p50_upper_us: Option<u32>,
    p95_upper_us: Option<u32>,
    max_us: Option<u32>,
}

#[derive(Serialize)]
pub(crate) struct Metrics {
    device_queue_full: u32,
    plugin_queue_full: u32,
    provider_queue_full: u32,
    timer_queue_full: u32,
    wake_inference: Timing,
    wake_inference_failed: u32,
    capture_skipped_frames: u32,
    #[cfg(target_os = "espidf")]
    wake_capture_samples: u32,
    #[cfg(target_os = "espidf")]
    wake_max_lag_samples: u32,
    voice_cancellations: u32,
    #[cfg(target_os = "espidf")]
    flash_writes: Timing,
    #[cfg(target_os = "espidf")]
    flash_writes_failed: u32,
}

pub(crate) fn snapshot() -> Metrics {
    Metrics {
        device_queue_full: DEVICE_QUEUE_FULL.load(Relaxed),
        plugin_queue_full: PLUGIN_QUEUE_FULL.load(Relaxed),
        provider_queue_full: PROVIDER_QUEUE_FULL.load(Relaxed),
        timer_queue_full: TIMER_QUEUE_FULL.load(Relaxed),
        wake_inference: WAKE.snapshot(),
        wake_inference_failed: WAKE_FAILURES.load(Relaxed),
        capture_skipped_frames: CAPTURE_SKIPPED_FRAMES.load(Relaxed),
        #[cfg(target_os = "espidf")]
        wake_capture_samples: WAKE_CAPTURE_SAMPLES.load(Relaxed),
        #[cfg(target_os = "espidf")]
        wake_max_lag_samples: WAKE_MAX_LAG_SAMPLES.load(Relaxed),
        voice_cancellations: VOICE_CANCELLATIONS.load(Relaxed),
        #[cfg(target_os = "espidf")]
        flash_writes: FLASH.snapshot(),
        #[cfg(target_os = "espidf")]
        flash_writes_failed: FLASH_FAILURES.load(Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_bounded_latency_percentiles_and_initially_has_no_samples() {
        let latency = Latency::new();
        assert_eq!(latency.snapshot().p95_upper_us, None);
        for _ in 0..11 {
            latency.observe(Duration::from_micros(10_400));
        }
        for _ in 0..8 {
            latency.observe(Duration::from_micros(65_000));
        }
        latency.observe(Duration::from_micros(245_000));
        let report = latency.snapshot();
        assert_eq!(report.count, 20);
        assert_eq!(report.p50_upper_us, Some(20_000));
        assert_eq!(report.p95_upper_us, Some(80_000));
        assert_eq!(report.max_us, Some(245_000));
    }
}

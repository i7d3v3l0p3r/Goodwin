#[cfg(target_arch = "wasm32")]
use alloc::string::String;
use serde::{Deserialize, Serialize};

pub const POLL_METHOD: &str = "timer.poll";
pub const MIN_TRUSTED_WALL_MS: u64 = 1_577_836_800_000;

/// Compare a spoken shortcut as complete words, ignoring ASR punctuation and case.
pub fn phrase_key(text: &str) -> String {
    let mut key = String::new();
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !key.is_empty() {
            key.push(' ');
        }
        key.push_str(&word.to_lowercase());
    }
    key
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Poll {
    pub played: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub alarm: Option<u64>,
    pub wake_after_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSample {
    pub elapsed_ms: u64,
    pub wall_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
use std::{
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// An epoch timestamp that advances with the monotonic clock for this process.
#[cfg(not(target_arch = "wasm32"))]
pub fn countdown_time_ms() -> u64 {
    let sample = clock_sample();
    sample.wall_ms.unwrap_or(sample.elapsed_ms)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn clock_sample() -> ClockSample {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    static EPOCH: OnceLock<(Instant, u64)> = OnceLock::new();
    let now = Instant::now();
    let start = ORIGIN.get_or_init(|| now);
    let wall = anchored_time(&EPOCH, now, || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|time| time.as_millis() as u64)
            // ESP can start with an unset RTC while SNTP connects.
            .filter(|&time| !cfg!(target_os = "espidf") || time >= MIN_TRUSTED_WALL_MS)
    });
    ClockSample {
        elapsed_ms: now.saturating_duration_since(*start).as_millis() as u64,
        wall_ms: (wall != 0).then_some(wall),
        session: None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn anchored_time(
    epoch: &OnceLock<(Instant, u64)>,
    now: Instant,
    wall_ms: impl FnOnce() -> Option<u64>,
) -> u64 {
    let (anchor, start) = match epoch.get() {
        Some(epoch) => epoch,
        None => {
            let Some(start) = wall_ms() else { return 0 };
            epoch.get_or_init(|| (now, start))
        }
    };
    start.saturating_add(now.saturating_duration_since(*anchor).as_millis() as u64)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn waits_for_the_first_clock_reading_then_ignores_wall_clock_steps() {
        let epoch = OnceLock::new();
        let start = Instant::now();
        assert_eq!(anchored_time(&epoch, start, || None), 0);
        assert_eq!(anchored_time(&epoch, start, || Some(50_000)), 50_000);
        assert_eq!(
            anchored_time(&epoch, start + Duration::from_secs(1), || Some(500_000)),
            51_000
        );
        assert_eq!(
            anchored_time(&epoch, start + Duration::from_secs(2), || Some(1_000)),
            52_000
        );
    }
}

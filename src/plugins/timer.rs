use std::time::{Duration, Instant};

use goodwin_plugin::timer::{Poll, Status};

const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Alarm {
    Queued(u64),
    Playing(u64),
}

pub(crate) struct Schedule {
    enabled: bool,
    next: Option<Instant>,
    alarm: Option<Alarm>,
    played: Option<u64>,
}

impl Schedule {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            next: enabled.then(Instant::now),
            alarm: None,
            played: None,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn next(&self) -> Option<Instant> {
        self.next
    }

    pub(crate) fn changed(&mut self) {
        if self.enabled && self.alarm.is_none() {
            self.next = Some(Instant::now());
        }
    }

    pub(crate) fn poll(&self) -> Poll {
        Poll {
            played: self.played,
        }
    }

    pub(crate) fn update(&mut self, status: Status) {
        self.played = None;
        self.alarm = status.alarm.map(Alarm::Queued);
        self.next = status
            .wake_after_ms
            .filter(|_| self.alarm.is_none())
            .map(|delay| Instant::now() + Duration::from_millis(delay.clamp(10, 86_400_000)));
    }

    pub(crate) fn retry(&mut self) {
        self.next = Some(Instant::now() + RETRY);
    }

    pub(crate) fn take_alarm(&mut self) -> Option<u64> {
        let Some(Alarm::Queued(id)) = self.alarm else {
            return None;
        };
        self.alarm = Some(Alarm::Playing(id));
        Some(id)
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn playing(&self) -> bool {
        matches!(self.alarm, Some(Alarm::Playing(_)))
    }

    pub(crate) fn finish(&mut self, id: u64, success: bool) {
        if self.alarm != Some(Alarm::Playing(id)) {
            return;
        }
        self.alarm = None;
        if success {
            self.played = Some(id);
            self.changed();
        } else {
            self.retry();
        }
    }
}

#[cfg(not(target_os = "espidf"))]
mod linux;
#[cfg(not(target_os = "espidf"))]
pub(super) use linux::{Delivery, Signal};

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::*;

    #[test]
    fn retries_failed_playback_and_only_acknowledges_the_playing_alarm() {
        let mut schedule = Schedule::new(true);
        schedule.update(Status {
            alarm: Some(42),
            wake_after_ms: Some(10),
        });
        assert_eq!(schedule.take_alarm(), Some(42));
        assert!(schedule.take_alarm().is_none());
        schedule.finish(41, true);
        assert!(schedule.next().is_none());
        schedule.finish(42, false);
        assert!(schedule.next().is_some());
        assert_eq!(schedule.poll().played, None);
        schedule.update(Status {
            alarm: Some(42),
            wake_after_ms: None,
        });
        assert_eq!(schedule.take_alarm(), Some(42));
        schedule.finish(42, true);
        assert_eq!(schedule.poll().played, Some(42));
        schedule.retry();
        assert_eq!(schedule.poll().played, Some(42));
        schedule.update(Status::default());
        assert_eq!(schedule.poll().played, None);
    }
}

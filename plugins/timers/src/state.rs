#[cfg(target_arch = "wasm32")]
use alloc::{borrow::ToOwned, string::String, vec::Vec};
#[cfg(target_arch = "wasm32")]
use core::cell::{Cell, RefCell};
use core::{error::Error, fmt, ops::DerefMut};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    path::PathBuf,
    sync::{Condvar, Mutex},
    time::Duration,
};

use goodwin_plugin::timer::{ClockSample, MIN_TRUSTED_WALL_MS, Poll, Status};
#[cfg(not(target_arch = "wasm32"))]
use goodwin_plugin::{Host, read_json, save_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_SECONDS: u64 = 7 * 24 * 3600;
const MAX_ACTIVE: usize = 32;
const MAX_COMPLETED: usize = 64;

#[derive(Debug)]
pub(super) enum ClockError {
    IdsExhausted,
    #[cfg(not(target_arch = "wasm32"))]
    Save(goodwin_error::Error),
}

impl fmt::Display for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdsExhausted => formatter.write_str("Timer IDs exhausted"),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Save(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for ClockError {
    #[cfg(not(target_arch = "wasm32"))]
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::IdsExhausted => None,
            Self::Save(error) => error.as_error().source(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub id: u64,
    name: Option<String>,
    seconds: u64,
    expires_at: u64,
    completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offline_remaining_ms: Option<u64>,
    #[serde(skip)]
    deadline: Option<u64>,
}

impl Timer {
    fn active(&self, time: u64) -> bool {
        !self.completed && self.deadline.is_some_and(|deadline| deadline > time)
    }

    pub fn voice(&self, time: u64) -> Value {
        let remaining = self
            .deadline
            .map_or(0, |deadline| deadline.saturating_sub(time));
        json!({"name": self.name, "seconds": self.seconds, "remainingSeconds": remaining.div_ceil(1000)})
    }

    pub fn view(&self) -> Value {
        json!({"id": self.id, "name": self.name, "seconds": self.seconds,
            "expiresAt": self.expires_at, "completed": self.completed})
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct State {
    next_id: u64,
    timers: Vec<Timer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_alarms: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offline_session: Option<u64>,
}

impl State {
    fn resolve(&mut self, sample: ClockSample) {
        for timer in self
            .timers
            .iter_mut()
            .filter(|timer| !timer.completed && timer.deadline.is_none())
        {
            if timer.expires_at < MIN_TRUSTED_WALL_MS && timer.offline_remaining_ms.is_none() {
                // Older firmware could store a boot-relative deadline with no checkpoint.
                timer.offline_remaining_ms = Some(timer.seconds.saturating_mul(1_000));
            }
            if timer.offline_remaining_ms.is_some()
                && self.offline_session.is_some()
                && self.offline_session == sample.session
            {
                timer.deadline = Some(timer.expires_at);
                continue;
            }
            let remaining = timer.offline_remaining_ms.or_else(|| {
                sample
                    .wall_ms
                    .map(|wall| timer.expires_at.saturating_sub(wall))
            });
            timer.deadline = remaining.map(|remaining| sample.elapsed_ms.saturating_add(remaining));
        }
    }

    fn checkpoint(&mut self, sample: ClockSample) -> bool {
        let mut changed = false;
        for timer in &mut self.timers {
            if let (Some(saved), Some(deadline)) = (timer.offline_remaining_ms, timer.deadline) {
                let remaining = deadline.saturating_sub(sample.elapsed_ms);
                if let Some(wall) = sample.wall_ms {
                    timer.expires_at = wall
                        .saturating_add(remaining)
                        .saturating_sub(sample.elapsed_ms.saturating_sub(deadline));
                    timer.offline_remaining_ms = None;
                    changed = true;
                } else if remaining != saved {
                    timer.offline_remaining_ms = Some(remaining);
                    changed = true;
                }
                if sample.wall_ms.is_none() && timer.expires_at != deadline {
                    timer.expires_at = deadline;
                    changed = true;
                }
            }
        }
        let session = sample.session.filter(|_| {
            self.timers
                .iter()
                .any(|timer| timer.offline_remaining_ms.is_some())
        });
        changed |= self.offline_session != session;
        self.offline_session = session;
        changed
    }

    fn next_deadline(&self) -> Option<u64> {
        self.timers
            .iter()
            .filter(|timer| !timer.completed)
            .filter_map(|timer| timer.deadline)
            .min()
    }

    fn finish(&mut self, time: u64) -> usize {
        let mut changed = 0;
        for timer in &mut self.timers {
            if !timer.completed && timer.deadline.is_some_and(|deadline| deadline <= time) {
                timer.completed = true;
                // Persist the alarm in the same state change as the completion.
                self.pending_alarms.push(timer.id);
                changed += 1;
            }
        }
        if changed > 0 {
            let mut completed: Vec<_> = self
                .timers
                .iter()
                .filter(|timer| timer.completed)
                .map(|timer| (timer.expires_at, timer.id))
                .collect();
            completed.sort_unstable();
            let obsolete = &completed[..completed.len().saturating_sub(MAX_COMPLETED)];
            self.timers
                .retain(|timer| !obsolete.contains(&(timer.expires_at, timer.id)));
        }
        changed
    }

    fn delay(&self, sample: ClockSample) -> Option<u64> {
        let delay = self
            .next_deadline()
            .map(|deadline| deadline.saturating_sub(sample.elapsed_ms));
        if sample.wall_ms.is_none() && self.timers.iter().any(|timer| !timer.completed) {
            return Some(delay.unwrap_or(1_000).min(1_000));
        }
        delay
    }
}

#[derive(Default)]
pub struct Filter {
    name: Option<String>,
    seconds: Option<u64>,
}

impl Filter {
    pub fn parse(value: &Value) -> Option<Self> {
        let name = value
            .get("name")
            .map(timer_name)
            .transpose()
            .ok()?
            .flatten();
        let seconds = match value.get("seconds") {
            Some(seconds) => Some(valid_seconds(seconds)?),
            None => None,
        };
        if value.get("name").is_some() && name.is_none() {
            return None;
        }
        Some(Self {
            name: name.as_deref().map(normalize_name),
            seconds,
        })
    }

    pub fn without_name(&self) -> Self {
        Self {
            name: None,
            seconds: self.seconds,
        }
    }
}

fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub struct Clock {
    #[cfg(not(target_arch = "wasm32"))]
    file: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    state: Mutex<State>,
    #[cfg(target_arch = "wasm32")]
    state: RefCell<State>,
    #[cfg(not(target_arch = "wasm32"))]
    changes: Condvar,
    #[cfg(target_arch = "wasm32")]
    dirty: Cell<bool>,
}

impl Clock {
    fn state_mut(&self) -> impl DerefMut<Target = State> + '_ {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.state.lock().expect("timer state")
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.state.borrow_mut()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(file: PathBuf) -> goodwin_error::Result<Self> {
        let mut state: State = read_json(&file)?;
        let time = sample();
        state.resolve(time);
        if (state.finish(time.elapsed_ms) > 0) | state.checkpoint(time) {
            save_json(&file, &state)?;
        }
        Ok(Self {
            file,
            state: Mutex::new(state),
            changes: Condvar::new(),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn from_state(mut state: State) -> Self {
        let time = sample();
        state.resolve(time);
        let dirty = (state.finish(time.elapsed_ms) != 0) | state.checkpoint(time);
        Self {
            state: RefCell::new(state),
            dirty: Cell::new(dirty),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn take_state(&self) -> Result<Option<Value>, serde_json::Error> {
        self.dirty
            .replace(false)
            .then(|| serde_json::to_value(&*self.state_mut()))
            .transpose()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn next_deadline(&self) -> Option<u64> {
        let time = sample();
        let mut state = self.state_mut();
        state.resolve(time);
        let timestamp = time.wall_ms.unwrap_or(time.elapsed_ms);
        if state.pending_alarms.is_empty() {
            state
                .delay(time)
                .map(|delay| timestamp.saturating_add(delay))
        } else {
            Some(timestamp)
        }
    }

    pub fn poll(&self, poll: Poll) -> Result<Status, ClockError> {
        self.poll_at(&mut self.state_mut(), poll, sample())
    }

    fn poll_at(
        &self,
        state: &mut State,
        poll: Poll,
        time: ClockSample,
    ) -> Result<Status, ClockError> {
        state.resolve(time);
        let acknowledged =
            poll.played.is_some() && poll.played == state.pending_alarms.first().copied();
        let due = state
            .next_deadline()
            .is_some_and(|deadline| deadline <= time.elapsed_ms);
        let synchronizing = time.wall_ms.is_some()
            && state
                .timers
                .iter()
                .any(|timer| timer.deadline.is_some() && timer.offline_remaining_ms.is_some());
        if acknowledged || due || synchronizing {
            let mut next = state.clone();
            if acknowledged {
                next.pending_alarms.remove(0);
            }
            if due {
                next.finish(time.elapsed_ms);
            }
            self.commit(state, next, time)?;
        }
        Ok(Status {
            alarm: state.pending_alarms.first().copied(),
            wake_after_ms: state.delay(time),
        })
    }

    pub fn list(&self) -> Value {
        let time = sample();
        let timestamp = time.wall_ms.unwrap_or(time.elapsed_ms);
        let mut state = self.state_mut();
        state.resolve(time);
        let timers = state
            .timers
            .iter()
            .map(|timer| {
                let mut view = timer.view();
                if !timer.completed {
                    if let Some(deadline) = timer.deadline {
                        if timer.offline_remaining_ms.is_some() {
                            let expires_at = time.wall_ms.map_or(deadline, |wall| {
                                wall.saturating_add(deadline.saturating_sub(time.elapsed_ms))
                            });
                            view["expiresAt"] = json!(expires_at);
                        }
                    } else {
                        view["waitingForClock"] = json!(true);
                    }
                }
                view
            })
            .collect::<Vec<_>>();
        json!({"now": timestamp, "timers": timers})
    }

    pub fn matches(&self, filter: &Filter) -> Vec<Timer> {
        let time = sample();
        let mut state = self.state_mut();
        state.resolve(time);
        state
            .timers
            .iter()
            .filter(|timer| {
                timer.active(time.elapsed_ms)
                    && filter
                        .seconds
                        .is_none_or(|seconds| timer.seconds == seconds)
                    && filter.name.as_ref().is_none_or(|name| {
                        timer
                            .name
                            .as_deref()
                            .is_some_and(|timer_name| normalize_name(timer_name) == *name)
                    })
            })
            .cloned()
            .collect()
    }

    pub fn create(&self, value: &Value) -> Result<Result<Timer, &'static str>, ClockError> {
        let Some(seconds) = valid_seconds(&value["seconds"]) else {
            return Ok(Err("invalid_timer_duration"));
        };
        let Ok(name) = timer_name(&value["name"]) else {
            return Ok(Err("invalid_timer_name"));
        };
        let mut state = self.state_mut();
        let time = sample();
        state.resolve(time);
        let running = state.timers.iter().filter(|timer| !timer.completed).count();
        if running + state.pending_alarms.len() >= MAX_ACTIVE {
            return Ok(Err("timer_limit"));
        }
        let mut next = state.clone();
        next.next_id = next
            .next_id
            .checked_add(1)
            .ok_or(ClockError::IdsExhausted)?;
        let timer = Timer {
            id: next.next_id,
            name,
            seconds,
            expires_at: time
                .wall_ms
                .unwrap_or(time.elapsed_ms)
                .saturating_add(seconds * 1_000),
            completed: false,
            offline_remaining_ms: time.wall_ms.is_none().then_some(seconds * 1_000),
            deadline: Some(time.elapsed_ms.saturating_add(seconds * 1_000)),
        };
        next.timers.push(timer.clone());
        self.commit(&mut state, next, time)?;
        Ok(Ok(timer))
    }

    pub fn cancel(&self, id: u64) -> Result<Option<Timer>, ClockError> {
        let mut state = self.state_mut();
        let time = sample();
        state.resolve(time);
        let Some(index) = state.timers.iter().position(|timer| {
            timer.id == id
                && !timer.completed
                && timer
                    .deadline
                    .is_none_or(|deadline| deadline > time.elapsed_ms)
        }) else {
            return Ok(None);
        };
        let mut next = state.clone();
        let timer = next.timers.remove(index);
        self.commit(&mut state, next, time)?;
        Ok(Some(timer))
    }

    fn commit(
        &self,
        state: &mut State,
        mut next: State,
        time: ClockSample,
    ) -> Result<(), ClockError> {
        next.checkpoint(time);
        #[cfg(not(target_arch = "wasm32"))]
        save_json(&self.file, &next).map_err(ClockError::Save)?;
        *state = next;
        #[cfg(not(target_arch = "wasm32"))]
        self.changes.notify_one();
        #[cfg(target_arch = "wasm32")]
        self.dirty.set(true);
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn schedule(&self, host: Host) {
        let mut state = self.state.lock().expect("timer state");
        let mut poll = Poll::default();
        loop {
            let delay = match self.poll_at(&mut state, poll, sample()) {
                Ok(Status {
                    alarm: Some(alarm), ..
                }) => {
                    poll.played = None;
                    drop(state);
                    let request = host.play_timer_signal();
                    state = self.state.lock().expect("timer state");
                    match request {
                        Ok(()) => {
                            // Legacy hosts confirm only that the request was sent.
                            poll.played = Some(alarm);
                            continue;
                        }
                        Err(error) => eprintln!("Request timer signal: {error:#}"),
                    }
                    Some(1_000)
                }
                Ok(status) => {
                    poll.played = None;
                    status.wake_after_ms
                }
                Err(error) => {
                    eprintln!("Save timer state: {error:#}");
                    Some(1_000)
                }
            };
            state = match delay {
                Some(delay) => {
                    self.changes
                        .wait_timeout(state, Duration::from_millis(delay.min(86_400_000)))
                        .expect("timer state")
                        .0
                }
                None => self.changes.wait(state).expect("timer state"),
            };
        }
    }
}

pub fn now() -> u64 {
    sample().elapsed_ms
}

#[cfg(target_arch = "wasm32")]
use super::wasm::sample;
#[cfg(not(target_arch = "wasm32"))]
use goodwin_plugin::timer::clock_sample as sample;

fn valid_seconds(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
}

fn timer_name(value: &Value) -> Result<Option<String>, ()> {
    if value.is_null() {
        return Ok(None);
    }
    let name = value.as_str().ok_or(())?.trim();
    if name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(());
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn startup_finishes_expired_timers_at_their_original_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("timers.json");
        let wall = sample().wall_ms.unwrap();
        let expiry = wall - 1000;
        let next = wall + 120_000;
        let timer = |id, expires_at| Timer {
            id,
            name: None,
            seconds: 120,
            expires_at,
            completed: false,
            deadline: None,
            offline_remaining_ms: None,
        };
        save_json(
            &path,
            &State {
                next_id: 2,
                timers: vec![timer(1, expiry), timer(2, next)],
                pending_alarms: Vec::new(),
                offline_session: None,
            },
        )
        .unwrap();
        let clock = Clock::load(path.clone()).unwrap();
        let timers = clock.list();
        assert_eq!(timers["timers"][0]["expiresAt"], expiry);
        assert_eq!(timers["timers"][0]["completed"], true);
        assert_eq!(timers["timers"][1]["expiresAt"], next);
        assert_eq!(timers["timers"][1]["completed"], false);
        let saved: State = read_json(&path).unwrap();
        assert!(saved.timers[0].completed);
        assert_eq!(saved.pending_alarms, [1]);
        drop(Clock::load(path.clone()).unwrap());
        let after_restart: State = read_json(&path).unwrap();
        assert_eq!(after_restart.pending_alarms, [1]);
    }

    #[test]
    fn completed_timers_from_older_versions_have_no_replay_pending() {
        let mut legacy: State = serde_json::from_value(json!({
            "next_id": 7,
            "timers": [{"id":7,"name":null,"seconds":20,"expiresAt":1000,"completed":true}]
        }))
        .unwrap();
        assert_eq!(legacy.finish(2000), 0);
        assert!(legacy.pending_alarms.is_empty());
    }

    #[test]
    fn playback_acknowledgement_is_durable_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("timers.json");
        let clock = Clock::load(path.clone()).unwrap();
        for _ in 0..2 {
            clock.create(&json!({"seconds":1})).unwrap().unwrap();
        }
        let mut state = clock.state_mut();
        let time = sample();
        let elapsed = ClockSample {
            elapsed_ms: time.elapsed_ms + 1_500,
            ..time
        };
        assert_eq!(
            clock
                .poll_at(&mut state, Poll::default(), elapsed)
                .unwrap()
                .alarm,
            Some(1)
        );
        assert_eq!(
            clock
                .poll_at(&mut state, Poll { played: Some(2) }, elapsed)
                .unwrap()
                .alarm,
            Some(1)
        );
        assert_eq!(
            clock
                .poll_at(&mut state, Poll { played: Some(1) }, elapsed)
                .unwrap()
                .alarm,
            Some(2)
        );
        assert_eq!(
            clock
                .poll_at(&mut state, Poll { played: Some(1) }, elapsed)
                .unwrap()
                .alarm,
            Some(2)
        );
        let saved: State = read_json(&path).unwrap();
        assert_eq!(saved.pending_alarms, [2]);
    }

    #[test]
    fn offline_timers_keep_elapsed_time_through_reload_and_resume_a_checkpoint_after_reboot() {
        let mut saved: State = serde_json::from_value(json!({
            "next_id": 1,
            "timers": [{"id":1,"name":null,"seconds":5,"expiresAt":5100,
                "offlineRemainingMs":5000,"completed":false}],
            "offline_session": 7,
        }))
        .unwrap();
        let old_boot = ClockSample {
            elapsed_ms: 2_100,
            wall_ms: None,
            session: Some(7),
        };
        saved.resolve(old_boot);
        assert_eq!(saved.delay(old_boot), Some(1_000));
        assert!(saved.checkpoint(old_boot));
        assert_eq!(saved.timers[0].offline_remaining_ms, Some(3_000));

        let snapshot = serde_json::to_value(&saved).unwrap();
        let mut reloaded: State = serde_json::from_value(snapshot.clone()).unwrap();
        reloaded.resolve(ClockSample {
            elapsed_ms: 2_800,
            ..old_boot
        });
        assert_eq!(reloaded.next_deadline(), Some(5_100));

        let mut restarted: State = serde_json::from_value(snapshot).unwrap();
        let new_boot = ClockSample {
            elapsed_ms: 100,
            wall_ms: None,
            session: Some(8),
        };
        restarted.resolve(new_boot);
        assert_eq!(restarted.next_deadline(), Some(3_100));
        let synchronized = ClockSample {
            elapsed_ms: 600,
            wall_ms: Some(MIN_TRUSTED_WALL_MS),
            ..new_boot
        };
        restarted.resolve(synchronized);
        assert!(restarted.checkpoint(synchronized));
        assert_eq!(restarted.timers[0].expires_at, MIN_TRUSTED_WALL_MS + 2_500);
        assert_eq!(restarted.next_deadline(), Some(3_100));
        assert_eq!(restarted.finish(3_100), 1);
        assert_eq!(restarted.pending_alarms, [1]);
    }

    #[test]
    fn saved_wall_clock_deadlines_wait_for_a_trusted_clock_after_reboot() {
        let mut state: State = serde_json::from_value(json!({
            "next_id": 1,
            "timers": [{"id":1,"name":null,"seconds":5,"expiresAt":MIN_TRUSTED_WALL_MS + 2000,"completed":false}]
        })).unwrap();
        let offline = ClockSample {
            elapsed_ms: 10_000,
            wall_ms: None,
            session: Some(1),
        };
        state.resolve(offline);
        assert!(state.next_deadline().is_none());
        assert_eq!(state.finish(offline.elapsed_ms), 0);
        let synchronized = ClockSample {
            wall_ms: Some(MIN_TRUSTED_WALL_MS + 3_000),
            ..offline
        };
        state.resolve(synchronized);
        assert_eq!(state.finish(synchronized.elapsed_ms), 1);
        assert_eq!(state.pending_alarms, [1]);
    }
}

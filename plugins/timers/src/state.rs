use std::{
    path::PathBuf,
    sync::{Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use goodwin_plugin::{Host, read_json, save_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_SECONDS: u64 = 7 * 24 * 3600;
const MAX_ACTIVE: usize = 32;
const MAX_COMPLETED: usize = 64;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub id: u64,
    name: Option<String>,
    seconds: u64,
    expires_at: u64,
    completed: bool,
}

impl Timer {
    fn active(&self, time: u64) -> bool {
        !self.completed && self.expires_at > time
    }

    pub fn voice(&self, time: u64) -> Value {
        json!({"name": self.name, "seconds": self.seconds, "remainingSeconds": self.expires_at.saturating_sub(time).div_ceil(1000)})
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct State {
    next_id: u64,
    timers: Vec<Timer>,
}

impl State {
    fn finish(&mut self, time: u64) -> usize {
        let mut changed = 0;
        for timer in &mut self.timers {
            if !timer.completed && timer.expires_at <= time {
                timer.completed = true;
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
    file: PathBuf,
    state: Mutex<State>,
    changes: Condvar,
}

impl Clock {
    pub fn load(file: PathBuf) -> Result<Self> {
        let mut state: State = read_json(&file)?;
        if state.finish(now()) > 0 {
            save_json(&file, &state)?;
        }
        Ok(Self {
            file,
            state: Mutex::new(state),
            changes: Condvar::new(),
        })
    }

    pub fn list(&self) -> Value {
        json!({"now": now(), "timers": self.state.lock().expect("timer state").timers})
    }

    pub fn matches(&self, filter: &Filter) -> Vec<Timer> {
        let time = now();
        let state = self.state.lock().expect("timer state");
        state
            .timers
            .iter()
            .filter(|timer| {
                timer.active(time)
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

    pub fn create(&self, value: &Value) -> Result<Result<Timer, &'static str>> {
        let Some(seconds) = valid_seconds(&value["seconds"]) else {
            return Ok(Err("invalid_timer_duration"));
        };
        let Ok(name) = timer_name(&value["name"]) else {
            return Ok(Err("invalid_timer_name"));
        };
        let mut state = self.state.lock().expect("timer state");
        let time = now();
        if state
            .timers
            .iter()
            .filter(|timer| timer.active(time))
            .count()
            >= MAX_ACTIVE
        {
            return Ok(Err("timer_limit"));
        }
        let mut next = state.clone();
        next.next_id = next.next_id.checked_add(1).context("Timer IDs exhausted")?;
        let timer = Timer {
            id: next.next_id,
            name,
            seconds,
            expires_at: time + seconds * 1000,
            completed: false,
        };
        next.timers.push(timer.clone());
        self.commit(&mut state, next)?;
        Ok(Ok(timer))
    }

    pub fn cancel(&self, id: u64) -> Result<Option<Timer>> {
        let mut state = self.state.lock().expect("timer state");
        let time = now();
        let Some(index) = state
            .timers
            .iter()
            .position(|timer| timer.id == id && timer.active(time))
        else {
            return Ok(None);
        };
        let mut next = state.clone();
        let timer = next.timers.remove(index);
        self.commit(&mut state, next)?;
        Ok(Some(timer))
    }

    fn commit(&self, state: &mut State, next: State) -> Result<()> {
        save_json(&self.file, &next)?;
        *state = next;
        self.changes.notify_one();
        Ok(())
    }

    pub fn schedule(&self, host: Host) {
        let mut state = self.state.lock().expect("timer state");
        loop {
            let time = now();
            let next_deadline = state
                .timers
                .iter()
                .filter(|timer| !timer.completed)
                .map(|timer| timer.expires_at)
                .min();
            let Some(deadline) = next_deadline else {
                state = self.changes.wait(state).expect("timer state");
                continue;
            };
            if deadline > time {
                let wait = Duration::from_millis((deadline - time).min(5_000));
                state = self
                    .changes
                    .wait_timeout(state, wait)
                    .expect("timer state")
                    .0;
                continue;
            }
            let mut next = state.clone();
            let finished = next.finish(time);
            if let Err(error) = self.commit(&mut state, next) {
                eprintln!("Save timer completion: {error:#}");
                state = self
                    .changes
                    .wait_timeout(state, Duration::from_secs(1))
                    .expect("timer state")
                    .0;
                continue;
            }
            drop(state);
            for _ in 0..finished {
                if let Err(error) = host.play_timer_signal() {
                    eprintln!("Request timer signal: {error:#}");
                }
            }
            state = self.state.lock().expect("timer state");
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_finishes_expired_timers_at_their_original_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("timers.json");
        let expiry = now() - 1000;
        let next = now() + 120_000;
        let timer = |id, expires_at| Timer {
            id,
            name: None,
            seconds: 120,
            expires_at,
            completed: false,
        };
        save_json(
            &path,
            &State {
                next_id: 2,
                timers: vec![timer(1, expiry), timer(2, next)],
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
    }
}

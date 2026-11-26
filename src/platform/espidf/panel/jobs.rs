use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;

use crate::panel_core::random_bytes;

const RESULT_LIFETIME: Duration = Duration::from_secs(60);
const RESULT_LIMIT: usize = 4;

pub(super) type Problem = (u16, &'static str);
pub(super) type Outcome<T> = Result<T, Problem>;

#[derive(Deserialize)]
pub(super) struct Ticket {
    pub operation: String,
}

struct Completed<T> {
    id: String,
    outcome: Outcome<T>,
    expires: Instant,
}

struct State<T> {
    active: Option<String>,
    completed: VecDeque<Completed<T>>,
}

pub(super) struct Jobs<T>(Arc<Mutex<State<T>>>);

impl<T> Default for Jobs<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            active: None,
            completed: VecDeque::new(),
        })))
    }
}

impl<T: Send + 'static> Jobs<T> {
    /// Prepare only short, nonblocking work here. The returned closure owns the job.
    pub(super) fn submit<F>(
        &self,
        name: &str,
        stack: usize,
        busy: Problem,
        prepare: impl FnOnce() -> Outcome<F>,
    ) -> Outcome<String>
    where
        F: FnOnce() -> Outcome<T> + Send + 'static,
    {
        let mut state = self.0.lock().expect("panel jobs");
        if state.active.is_some() {
            return Err(busy);
        }
        let id = URL_SAFE_NO_PAD.encode(random_bytes::<16>().map_err(|_| (500, "internal"))?);
        let work = prepare()?;
        let worker_state = Arc::clone(&self.0);
        state.active = Some(id.clone());
        if let Err(error) = thread::Builder::new()
            .name(name.to_owned())
            .stack_size(stack)
            .spawn(move || {
                let outcome = work();
                worker_state.lock().expect("panel jobs").complete(outcome);
            })
        {
            state.active = None;
            log::warn!("Start {name}: {error}");
            return Err((503, "busy"));
        }
        Ok(id)
    }

    pub(super) fn take(&self, id: &str, expired: &'static str) -> Outcome<Option<T>> {
        let mut state = self.0.lock().expect("panel jobs");
        if state.active.as_deref() == Some(id) {
            return Ok(None);
        }
        let now = Instant::now();
        state.completed.retain(|result| result.expires > now);
        let index = state
            .completed
            .iter()
            .position(|result| result.id == id)
            .ok_or((404, expired))?;
        state
            .completed
            .remove(index)
            .expect("completed panel job")
            .outcome
            .map(Some)
    }
}

impl<T> State<T> {
    fn complete(&mut self, outcome: Outcome<T>) {
        let id = self.active.take().expect("active panel job");
        let now = Instant::now();
        self.completed.retain(|result| result.expires > now);
        if self.completed.len() == RESULT_LIMIT {
            self.completed.pop_front();
        }
        self.completed.push_back(Completed {
            id,
            outcome,
            expires: now + RESULT_LIFETIME,
        });
    }
}

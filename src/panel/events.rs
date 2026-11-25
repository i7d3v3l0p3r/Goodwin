use std::{
    future::pending,
    time::{Duration, SystemTime},
};

use axum::{
    extract::State,
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream;
use serde::Serialize;
use tokio::sync::watch;

use super::{Panel, auth::access_state};

use crate::panel_core::AccessState;

#[cfg(test)]
mod tests;

pub(super) async fn subscribe(State(panel): State<Panel>, headers: HeaderMap) -> impl IntoResponse {
    let sessions = panel.auth.lock().await.changes.subscribe();
    let events = Events {
        runtime: panel.app.changes(),
        config: panel.app.configuration().changes(),
        plugins: panel.app.plugins.catalog_changes(),
        chat: panel.app.chat_changes(),
        shutdown: panel.app.panel_shutdown(),
        sessions,
        panel,
        headers,
        access: None,
        expires: None,
        snapshot_pending: false,
        chat_cursor: None,
    };
    let events = stream::unfold(events, |mut events| async move {
        events.next().await.map(|event| (event, events))
    });
    (
        [("x-accel-buffering", "no")],
        Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(20))),
    )
}

struct Events {
    panel: Panel,
    headers: HeaderMap,
    runtime: watch::Receiver<()>,
    config: watch::Receiver<()>,
    plugins: watch::Receiver<()>,
    chat: watch::Receiver<()>,
    shutdown: watch::Receiver<bool>,
    sessions: watch::Receiver<()>,
    access: Option<AccessState>,
    expires: Option<SystemTime>,
    snapshot_pending: bool,
    chat_cursor: Option<(u64, usize)>,
}

impl Events {
    async fn next(&mut self) -> Option<Result<Event, axum::Error>> {
        loop {
            if *self.shutdown.borrow() {
                return None;
            }
            let authorized = self.access.is_some_and(|access| access.authorized);
            tokio::select! {
                biased;
                _ = self.shutdown.changed() => return None,
                _ = self.sessions.changed() => {}
                _ = wait_for_expiry(self.expires) => {}
                _ = self.config.changed() => self.snapshot_pending = authorized,
                _ = async {}, if self.access.is_none() => {}
                _ = async {}, if self.snapshot_pending && authorized => {
                    self.snapshot_pending = false;
                    return Some(self.snapshot());
                }
                _ = self.runtime.changed(), if authorized => return Some(self.snapshot()),
                _ = self.plugins.changed(), if authorized => return Some(self.snapshot()),
                _ = self.chat.changed(), if authorized => {
                    if let Some(delta) = self.panel.app.chat_delta(&mut self.chat_cursor) {
                        return Some(json_event("chat_delta", delta));
                    }
                    continue;
                }
            }
            let (access, expires) = access_state(&self.panel, &self.headers).await;
            self.expires = expires;
            if self.access != Some(access) {
                self.access = Some(access);
                self.snapshot_pending = access.authorized;
                self.chat_cursor = None;
                return Some(json_event("access", access));
            }
        }
    }

    fn snapshot(&mut self) -> Result<Event, axum::Error> {
        self.runtime.borrow_and_update();
        self.plugins.borrow_and_update();
        let snapshot = self.panel.app.snapshot();
        self.chat_cursor = snapshot.chat_cursor();
        json_event("state", snapshot)
    }
}

async fn wait_for_expiry(expires: Option<SystemTime>) {
    if let Some(expires) = expires {
        tokio::time::sleep(
            expires
                .duration_since(SystemTime::now())
                .unwrap_or_default(),
        )
        .await;
    } else {
        pending().await
    }
}

fn json_event(name: &'static str, data: impl Serialize) -> Result<Event, axum::Error> {
    let json = goodwin_json::to_string(&data).map_err(axum::Error::new)?;
    Ok(Event::default().event(name).data(json))
}

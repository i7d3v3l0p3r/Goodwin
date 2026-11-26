use tokio::sync::{mpsc, oneshot};

use goodwin_error::Result;
use goodwin_plugin::timer::{Poll, Status};

use super::Schedule;

#[derive(Debug, Default)]
pub(crate) struct Signal(Option<oneshot::Sender<bool>>);

impl Signal {
    pub(crate) fn cancelled(&self) -> bool {
        self.0.as_ref().is_some_and(oneshot::Sender::is_closed)
    }

    pub(crate) fn finish(self, success: bool) {
        if let Some(completion) = self.0 {
            let _ = completion.send(success);
        }
    }
}

pub(crate) struct Delivery {
    schedule: Schedule,
    playing: Option<(u64, oneshot::Receiver<bool>)>,
}

impl Delivery {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            schedule: Schedule::new(enabled),
            playing: None,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.schedule.enabled()
    }

    pub(crate) fn changed(&mut self) {
        self.schedule.changed();
    }

    pub(crate) async fn wait(&mut self) -> Poll {
        if let Some((id, completion)) = &mut self.playing {
            let success = completion.await.unwrap_or(false);
            self.schedule.finish(*id, success);
            self.playing = None;
        }
        let Some(next) = self.schedule.next() else {
            return std::future::pending().await;
        };
        tokio::time::sleep_until(next.into()).await;
        self.schedule.poll()
    }

    pub(crate) fn update(&mut self, result: Result<Status>, sound: &mpsc::Sender<Signal>) {
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                log::warn!("Poll plugin timer alarms: {error:#}");
                self.schedule.retry();
                return;
            }
        };
        self.schedule.update(status);
        if let Some(id) = self.schedule.take_alarm() {
            let (send, receive) = oneshot::channel();
            if sound.try_send(Signal(Some(send))).is_ok() {
                self.playing = Some((id, receive));
            } else {
                self.schedule.finish(id, false);
            }
        }
    }
}

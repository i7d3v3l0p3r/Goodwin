use super::{Event, JobResult, Spawn, process};
use crate::{profile, types::Launch};
use goodwin_vpn::subscription::{DEFAULT_PROBE, Request};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf, sync::mpsc};

pub(super) const DEFAULT_INTERVAL: u64 = 300;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum Source {
    Direct(String), // Existing profile.json files store a direct link as a string.
    Subscription(Subscription),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Subscription {
    pub source: String,
    pub resolved: Option<String>,
    pub interval_seconds: u64,
}
impl Source {
    pub fn interval(&self) -> Option<u64> {
        match self {
            Self::Subscription(s) => Some(s.interval_seconds.clamp(5, 86400)),
            Self::Direct(_) => None,
        }
    }
}

pub(super) enum Mode {
    Apply,
    Restore,
    Check(Option<PathBuf>),
}

pub(super) enum Outcome {
    Healthy,
    Replace(Source),
}

#[derive(Deserialize)]
struct Settings {
    probe_url: Option<String>,
}

pub(super) struct Workflow {
    pub events: mpsc::Sender<Event>,
    pub id: u64,
    pub password: String,
}

impl Workflow {
    fn open(&self, what: Spawn) -> JobResult<process::Pipes> {
        let (sender, receiver) = mpsc::channel();
        self.events
            .send(Event::Spawn(self.id, what, sender))
            .map_err(|_| "vpn_engine_failed")?;
        receiver.recv().unwrap_or(Err("vpn_engine_failed"))
    }

    fn network(&self, request: Request) -> JobResult<Vec<String>> {
        let error = match request {
            Request::Probe { .. } => "vpn_probe_failed",
            Request::Resolve(_) => "vpn_subscription_failed",
        };
        let pipes = self.open(Spawn::Network)?;
        serde_json::to_writer(pipes.input, &request).map_err(|_| error)?;
        serde_json::from_reader(pipes.output.take(256 * 1024)).map_err(|_| error)
    }

    fn probe(&self, socket: PathBuf, url: &str) -> JobResult<()> {
        self.network(Request::Probe {
            socket,
            password: self.password.clone(),
            url: url.to_owned(),
        })?;
        Ok(())
    }

    fn candidate(&self, link: &str) -> JobResult<PathBuf> {
        let profile = profile::outbound(link).map_err(|_| "invalid_vless_profile")?;
        let pipes = self.open(Spawn::Worker(profile.worker()))?;
        process::handshake(
            pipes,
            &Launch {
                profile,
                password: self.password.clone(),
            },
        )
    }

    pub fn run(&self, desired: Source, mode: Mode) -> JobResult<Outcome> {
        let mut subscription = match desired {
            Source::Subscription(s) => s,
            Source::Direct(link) => {
                self.candidate(&link)?;
                return Ok(Outcome::Replace(Source::Direct(link)));
            }
        };
        let settings: Settings =
            goodwin_plugin::settings::read_settings().map_err(|_| "vpn_probe_failed")?;
        let target = settings.probe_url.as_deref().unwrap_or(DEFAULT_PROBE);
        goodwin_vpn::subscription::web_url(target).map_err(|_| "vpn_probe_failed")?;
        if let Mode::Check(Some(socket)) = &mode
            && self.probe(socket.clone(), target).is_ok()
        {
            return Ok(Outcome::Healthy);
        }
        let verify = |link: &str| {
            self.candidate(link)
                .and_then(|socket| self.probe(socket, target))
        };
        if matches!(mode, Mode::Restore | Mode::Check(None))
            && let Some(link) = &subscription.resolved
            && verify(link).is_ok()
        {
            return Ok(Outcome::Replace(Source::Subscription(subscription)));
        }
        let mut error = "invalid_vless_profile";
        for link in self.network(Request::Resolve(subscription.source.clone()))? {
            match verify(&link) {
                Ok(()) => {
                    subscription.resolved = Some(link);
                    return Ok(Outcome::Replace(Source::Subscription(subscription)));
                }
                Err(reason) => error = reason,
            }
        }
        Err(error)
    }
}

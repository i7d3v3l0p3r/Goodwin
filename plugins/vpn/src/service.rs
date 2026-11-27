mod process;
mod workflow;

use goodwin_error::{Context, Result};
use goodwin_plugin::{HttpRequest, HttpResponse, Plugin};
use process::{Pipes, Process, Worker};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use workflow::{DEFAULT_INTERVAL, Mode, Outcome, Source, Subscription, Workflow};

type JobResult<T> = std::result::Result<T, &'static str>;
type Reply = mpsc::Sender<HttpResponse>;

enum Spawn {
    Worker(&'static str),
    Network,
}
enum Event {
    Http(HttpRequest, Reply),
    Status(Reply),
    Spawn(u64, Spawn, mpsc::Sender<JobResult<Pipes>>),
    Finished(u64, JobResult<Outcome>),
    Stop,
}

struct Pending {
    desired: Source,
    checking: bool,
    worker: Option<Worker>,
    helper: Option<Process>,
    reply: Option<Reply>,
}

struct Routing {
    events: mpsc::Sender<Event>,
    password: String,
    socket: PathBuf,
    profile: PathBuf,
    saved: Option<Source>,
    worker: Option<Worker>,
    pending: Option<Pending>,
    generation: u64,
    next_check: Option<Instant>,
    error: Option<&'static str>,
    unreachable: bool,
    unreadable: bool,
}

impl Routing {
    fn publish(&self, worker: Option<&Worker>) -> Result<()> {
        if let Some(worker) = worker {
            let socket = worker.socket();
            let next = socket.with_file_name("publish");
            symlink(socket.canonicalize()?, &next)?;
            fs::rename(&next, &self.socket).inspect_err(|_| {
                let _ = fs::remove_file(next);
            })?;
        } else if let Err(e) = fs::remove_file(&self.socket)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e.into());
        }
        Ok(())
    }

    fn replace(&mut self, worker: Option<Worker>, source: Option<Source>) -> Result<()> {
        self.publish(worker.as_ref())?;
        if (source != self.saved || source.is_none())
            && let Err(e) = goodwin_plugin::save_json(&self.profile, &source)
        {
            self.publish(self.worker.as_ref())?;
            return Err(e);
        }
        self.worker = worker;
        self.saved = source;
        self.unreadable = false;
        self.unreachable = false;
        Ok(())
    }

    fn schedule(&mut self) {
        self.next_check = self
            .saved
            .as_ref()
            .and_then(Source::interval)
            .map(|secs| Instant::now() + Duration::from_secs(secs));
    }

    fn status(&mut self, submitted: bool) -> HttpResponse {
        let active = self.worker.as_mut().is_some_and(Worker::alive);
        let mut body =
            json!({"configured":self.saved.is_some() || self.unreadable, "active":active});
        if self.unreachable {
            body["unreachable"] = json!(true);
        }
        if let Some(interval) = self.saved.as_ref().and_then(Source::interval) {
            body["intervalSeconds"] = json!(interval);
        }
        if let Some(job) = &self.pending {
            body["pending"] = json!(true);
            if job.checking {
                body["checking"] = json!(true);
            }
        }
        if let Some(error) = self.error {
            body["error"] = json!(error);
        }
        let code = if submitted && self.pending.is_some() {
            202
        } else if submitted && self.error.is_some() {
            409
        } else {
            200
        };
        HttpResponse::json(code, body)
    }

    fn start(&mut self, desired: Source, mode: Mode, reply: Option<Reply>) {
        self.generation += 1;
        self.next_check = None;
        self.error = None;
        self.pending = Some(Pending {
            desired: desired.clone(),
            checking: !matches!(mode, Mode::Apply) && desired.interval().is_some(),
            worker: None,
            helper: None,
            reply,
        });
        let job = Workflow {
            id: self.generation,
            events: self.events.clone(),
            password: self.password.clone(),
        };
        if thread::Builder::new()
            .name("vpn-profile".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let result = job.run(desired, mode);
                let _ = job.events.send(Event::Finished(job.id, result));
            })
            .is_err()
        {
            self.finish(Err("vpn_engine_failed"));
        }
    }

    fn finish(&mut self, outcome: JobResult<Outcome>) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        self.error = outcome
            .and_then(|outcome| match outcome {
                Outcome::Healthy => Ok(()),
                Outcome::Replace(source) if pending.worker.as_mut().is_some_and(Worker::alive) => {
                    self.replace(pending.worker.take(), Some(source))
                        .map_err(|_| "vpn_engine_failed")
                }
                Outcome::Replace(_) => Err("vpn_engine_failed"),
            })
            .err();
        if pending.checking {
            self.unreachable = self.error.is_some();
        }
        self.schedule();
        if let Some(reply) = pending.reply {
            let _ = reply.send(self.status(true));
        }
    }

    fn spawn(&mut self, what: Spawn) -> Result<Pipes> {
        let job = self.pending.as_mut().context("VPN operation cancelled")?;
        job.helper.take();
        match what {
            Spawn::Worker(name) => {
                job.worker.take();
                let (worker, pipes) = Worker::new(&self.socket, name)?;
                job.worker = Some(worker);
                Ok(pipes)
            }
            Spawn::Network => {
                let (helper, pipes) = process::spawn(&["--subscription".as_ref()], PathBuf::new())?;
                job.helper = Some(helper);
                Ok(pipes)
            }
        }
    }

    fn check(&mut self) {
        if let Some(source @ Source::Subscription(_)) = self.saved.clone() {
            let socket = self
                .worker
                .as_mut()
                .and_then(|w| w.alive().then(|| w.socket()));
            self.start(source, Mode::Check(socket), None);
        }
    }

    fn http(&mut self, request: HttpRequest, reply: Reply) -> Result<()> {
        let response = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/profile") => self.status(false),
            ("DELETE", "/profile") => {
                self.pending.take();
                self.replace(None, None)?;
                self.next_check = None;
                self.error = None;
                self.status(false)
            }
            ("PUT", "/monitoring") => {
                let Some(Source::Subscription(mut subscription)) = self.saved.clone() else {
                    let _ = reply.send(HttpResponse::empty(409));
                    return Ok(());
                };
                if let Some(interval) = interval(&request.body) {
                    if self.pending.as_ref().is_some_and(|job| !job.checking) {
                        let _ = reply.send(HttpResponse::empty(409));
                        return Ok(());
                    }
                    subscription.interval_seconds = interval;
                    let source = Source::Subscription(subscription);
                    goodwin_plugin::save_json(&self.profile, &source)?;
                    self.saved = Some(source);
                    self.pending.take();
                    self.schedule();
                    self.status(false)
                } else {
                    invalid("invalid_vpn_interval")
                }
            }
            ("PUT", "/profile") => {
                let link = request
                    .body
                    .get("link")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let desired = if super::profile::outbound(link).is_ok() {
                    Source::Direct(link.to_owned())
                } else if goodwin_vpn::subscription::web_url(link).is_ok() {
                    let seconds = if request.body.get("intervalSeconds").is_some() {
                        interval(&request.body)
                    } else {
                        Some(DEFAULT_INTERVAL)
                    };
                    let Some(interval_seconds) = seconds else {
                        let _ = reply.send(invalid("invalid_vpn_interval"));
                        return Ok(());
                    };
                    Source::Subscription(Subscription {
                        source: link.to_owned(),
                        resolved: None,
                        interval_seconds,
                    })
                } else {
                    let _ = reply.send(invalid("invalid_vless_profile"));
                    return Ok(());
                };
                if self.pending.as_ref().is_some_and(|p| p.desired == desired) {
                    self.status(true)
                } else {
                    self.start(desired, Mode::Apply, Some(reply));
                    return Ok(());
                }
            }
            _ => HttpResponse::empty(404),
        };
        let _ = reply.send(response);
        Ok(())
    }

    fn run(&mut self, inbox: mpsc::Receiver<Event>) {
        loop {
            let event = if let Some(deadline) = self.next_check {
                if Instant::now() >= deadline {
                    self.check();
                    continue;
                }
                match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.check();
                        continue;
                    }
                    Err(_) => break,
                }
            } else if let Ok(event) = inbox.recv() {
                event
            } else {
                break;
            };
            match event {
                Event::Http(request, reply) => {
                    if self.http(request, reply.clone()).is_err() {
                        if self.pending.is_none() {
                            self.schedule();
                        }
                        let _ = reply.send(HttpResponse::json(
                            500,
                            json!({"error":"vpn_engine_failed"}),
                        ));
                    }
                }
                Event::Status(reply) => {
                    let _ = reply.send(self.status(true));
                }
                Event::Spawn(id, what, reply) if id == self.generation => {
                    let _ = reply.send(self.spawn(what).map_err(|_| "vpn_engine_failed"));
                }
                Event::Finished(id, result) if id == self.generation => self.finish(result),
                Event::Stop => break,
                _ => (),
            }
        }
    }
}

fn interval(body: &Value) -> Option<u64> {
    body.get("intervalSeconds")?
        .as_u64()
        .filter(|v| (5..=86400).contains(v))
}
fn invalid(error: &str) -> HttpResponse {
    HttpResponse::json(400, json!({"error":error}))
}

struct Vpn(mpsc::Sender<Event>);
impl Plugin for Vpn {
    fn registration(&self) -> Value {
        let routes = [
            ("GET", "/profile"),
            ("PUT", "/profile"),
            ("DELETE", "/profile"),
            ("PUT", "/monitoring"),
        ]
        .map(|(method, path)| json!({"method":method,"path":path}));
        json!({"apiVersion":1,"pages":[{"id":"profile","path":"","title":{"en":"VLESS VPN","ru":"VLESS VPN"},"navigation":true}],"routes":routes})
    }
    fn http(&mut self, request: HttpRequest) -> Result<HttpResponse> {
        let submit = request.method == "PUT" && request.path == "/profile";
        let (reply, receiver) = mpsc::channel();
        self.0
            .send(Event::Http(request, reply))
            .map_err(|_| goodwin_error::report!("VPN stopped"))?;
        if !submit {
            return Ok(receiver.recv()?);
        }
        match receiver.recv_timeout(Duration::from_millis(120)) {
            Ok(response) => Ok(response),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (reply, receiver) = mpsc::channel();
                self.0
                    .send(Event::Status(reply))
                    .map_err(|_| goodwin_error::report!("VPN stopped"))?;
                Ok(receiver.recv()?)
            }
            Err(e) => Err(e.into()),
        }
    }
}

pub(super) fn run() -> Result<()> {
    let profile = goodwin_plugin::data_dir()?.join("profile.json");
    let saved = goodwin_plugin::read_json::<Option<Source>>(&profile);
    let (events, inbox) = mpsc::channel();
    let mut routing = Routing {
        password: std::env::var("GOODWIN_VPN_PASSWORD").context("Missing VPN authentication")?,
        socket: std::env::var_os("GOODWIN_VPN_SOCKET")
            .context("Missing VPN socket")?
            .into(),
        error: saved.is_err().then_some("invalid_vless_profile"),
        unreadable: saved.is_err(),
        unreachable: false,
        saved: saved.unwrap_or(None),
        profile,
        events: events.clone(),
        worker: None,
        pending: None,
        generation: 0,
        next_check: None,
    };
    if let Some(source) = routing.saved.clone() {
        routing.start(source, Mode::Restore, None);
    }
    let sdk = thread::Builder::new()
        .name("vpn-rpc".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = goodwin_plugin::run(Vpn(events.clone()));
            let _ = events.send(Event::Stop);
            result
        })?;
    routing.run(inbox);
    sdk.join()
        .map_err(|_| goodwin_error::report!("VPN RPC stopped"))?
}

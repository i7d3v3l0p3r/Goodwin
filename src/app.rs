use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use goodwin_plugin::provider::Capability;
use serde::Serialize;
use tokio::sync::watch;

use crate::{
    config::{Config, ConfigStore, RequestSettings, Settings},
    plugins::{Plugins, View as PluginView},
    providers::Provider,
    station::{Indicators, Player},
    wake::{PendingModel, WakeModels},
    wake_catalog::Selection,
};

mod voice;

pub(crate) use voice::VOICE_INSTRUCTIONS;

const CHAT_INSTRUCTIONS: &str =
    "You are Goodwin, a home assistant. Reply concisely in the user's language.";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Chat,
    Wake,
    ManualTest,
    WakeTest,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    AwaitingWake,
    Recording,
    Transcribing,
    Thinking,
    Synthesizing,
    Playing,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Busy,
    Microphone,
    MicrophoneMuted,
    Model,
    ProviderUnavailable,
    ProviderFailed,
    Playback,
    NoSpeech,
    NoWake,
}

#[derive(Clone, Copy, Serialize)]
pub struct Timing {
    stage: Stage,
    ms: u64,
}

#[derive(Clone, Serialize)]
pub struct Report {
    source: Source,
    stage: Stage,
    timings: Vec<Timing>,
    error: Option<Failure>,
}

struct Turn {
    id: u64,
    started: Instant,
    report: Report,
    request: RequestSettings,
    wake_word: Option<String>,
}

pub(crate) struct ActiveTurn {
    pub id: u64,
    pub source: Source,
    pub stage: Stage,
    pub started: Instant,
    pub sensitivity: f32,
    pub wake_gain: Option<f32>,
    pub transcription_gain: Option<f32>,
}

#[derive(Clone, Serialize)]
pub struct Message {
    id: u64,
    role: &'static str,
    text: String,
}

#[derive(Default)]
struct Runtime {
    next_id: u64,
    next_message: u64,
    voice_turn: u64,
    active: Option<Turn>,
    last: Option<Report>,
    microphone: bool,
    model: bool,
    wake_model: Option<Selection>,
    messages: VecDeque<Message>,
}

pub struct App {
    pub dir: PathBuf,
    config: ConfigStore,
    runtime: Mutex<Runtime>,
    microphone_mute: watch::Sender<bool>,
    pub player: Player,
    pub(crate) indicators: Indicators,
    pub plugins: Plugins,
    pub(crate) default_microphone_gain: f32,
    pub(crate) wake_models: WakeModels,
}

#[derive(Serialize)]
pub struct Snapshot {
    settings: Settings,
    system_prompt: String,
    microphone: bool,
    default_microphone_gain: f32,
    model: bool,
    wake_model: Option<Selection>,
    active: Option<Report>,
    last: Option<Report>,
    messages: VecDeque<Message>,
    plugins: Vec<PluginView>,
}

impl App {
    pub(crate) fn new(
        dir: PathBuf,
        config: Config,
        player: Player,
        default_microphone_gain: f32,
    ) -> Arc<Self> {
        let plugins = Plugins::new(&dir, player.clone());
        let (microphone_mute, _) = watch::channel(config.settings.microphone_muted);
        let config = ConfigStore::new(&dir, config);
        let wake_models = WakeModels::new(&dir);
        Arc::new(Self {
            dir,
            config,
            runtime: Mutex::new(Runtime::default()),
            microphone_mute,
            player,
            indicators: Indicators::new(),
            plugins,
            default_microphone_gain,
            wake_models,
        })
    }

    pub(crate) fn configuration(&self) -> &ConfigStore {
        &self.config
    }

    fn runtime(&self) -> MutexGuard<'_, Runtime> {
        self.runtime.lock().expect("runtime mutex")
    }

    pub fn snapshot(&self) -> Snapshot {
        let runtime = self.runtime();
        let RequestSettings {
            settings,
            system_prompt,
        } = self.config.request_settings();
        Snapshot {
            settings,
            system_prompt,
            microphone: runtime.microphone,
            default_microphone_gain: self.default_microphone_gain,
            model: runtime.model,
            wake_model: runtime.wake_model.clone(),
            active: runtime.active.as_ref().map(|turn| turn.report.clone()),
            last: runtime.last.clone(),
            messages: runtime.messages.clone(),
            plugins: self.plugins.catalog(),
        }
    }

    pub(crate) fn microphone_mute(&self) -> watch::Receiver<bool> {
        self.microphone_mute.subscribe()
    }

    pub(crate) fn set_microphone_muted(&self, muted: bool) -> anyhow::Result<()> {
        let mut runtime = self.runtime();
        if *self.microphone_mute.borrow() == muted {
            return Ok(());
        }
        self.config.update_microphone_muted(muted)?;
        if muted {
            self.end_capture(&mut runtime, Failure::MicrophoneMuted);
        }
        self.microphone_mute.send_replace(muted);
        Ok(())
    }

    pub(crate) fn microphone_connected(&self, mute: &watch::Receiver<bool>) -> bool {
        let mut runtime = self.runtime();
        // A stream opened before a mute/unmute cycle must reconnect before delivering audio.
        if mute.has_changed().unwrap_or(true) || *mute.borrow() {
            return false;
        }
        runtime.microphone = true;
        true
    }

    pub(crate) fn microphone_disconnected(&self) {
        self.end_capture(&mut self.runtime(), Failure::Microphone);
    }

    fn end_capture(&self, runtime: &mut Runtime, error: Failure) {
        runtime.microphone = false;
        if let Some(id) = runtime.active.as_ref().and_then(|turn| {
            matches!(turn.report.stage, Stage::AwaitingWake | Stage::Recording).then_some(turn.id)
        }) {
            self.finish_turn(runtime, id, Some(error));
        }
    }

    pub fn set_model(&self, loaded: bool, selection: Option<Selection>) {
        let mut runtime = self.runtime();
        runtime.model = loaded;
        runtime.wake_model = selection;
    }

    pub(crate) fn take_pending_wake_model(&self) -> Option<PendingModel> {
        let mut runtime = self.runtime();
        if runtime
            .active
            .as_ref()
            .is_some_and(|turn| turn.report.stage == Stage::AwaitingWake)
        {
            return None;
        }
        let pending = self.wake_models.take_pending()?;
        runtime.model = true;
        runtime.wake_model = Some(pending.selection.clone());
        Some(pending)
    }

    pub(crate) fn active(&self) -> Option<ActiveTurn> {
        self.runtime().active.as_ref().map(|turn| ActiveTurn {
            id: turn.id,
            source: turn.report.source,
            stage: turn.report.stage,
            started: turn.started,
            sensitivity: turn.request.settings.sensitivity,
            wake_gain: turn.request.settings.wake_gain,
            transcription_gain: turn.request.settings.transcription_gain,
        })
    }

    pub fn start(&self, source: Source) -> Result<u64, Failure> {
        let request = self.config.request_settings();
        let mut runtime = self.runtime();
        if source != Source::Chat && *self.microphone_mute.borrow() {
            return Err(Failure::MicrophoneMuted);
        }
        if runtime.active.is_some() {
            return Err(Failure::Busy);
        }
        if source != Source::Chat && !runtime.microphone {
            return Err(Failure::Microphone);
        }
        if matches!(source, Source::Wake | Source::WakeTest) && !runtime.model {
            return Err(Failure::Model);
        }
        if source == Source::Wake && !request.settings.listening {
            return Err(Failure::Busy);
        }
        runtime.next_id += 1;
        let id = runtime.next_id;
        let stage = match source {
            Source::Chat => Stage::Thinking,
            Source::WakeTest => Stage::AwaitingWake,
            _ => Stage::Recording,
        };
        runtime.active = Some(Turn {
            id,
            started: Instant::now(),
            report: Report {
                source,
                stage,
                timings: Vec::new(),
                error: None,
            },
            request,
            wake_word: runtime
                .wake_model
                .as_ref()
                .map(|model| model.wake_word.clone()),
        });
        self.indicators.stage(source, stage);
        Ok(id)
    }

    pub fn stage(&self, id: u64, stage: Stage) {
        let mut runtime = self.runtime();
        if let Some(turn) = runtime.active.as_mut().filter(|turn| turn.id == id) {
            turn.record_timing();
            let wake_test_done =
                turn.report.stage == Stage::AwaitingWake && stage != Stage::AwaitingWake;
            turn.report.stage = stage;
            self.indicators.stage(turn.report.source, stage);
            if wake_test_done {
                self.wake_models.changed.notify_one();
            }
        }
    }

    pub fn finish(&self, id: u64, error: Option<Failure>) {
        self.finish_turn(&mut self.runtime(), id, error);
    }

    fn finish_turn(&self, runtime: &mut Runtime, id: u64, error: Option<Failure>) {
        if runtime.active.as_ref().is_some_and(|turn| turn.id == id) {
            let mut turn = runtime.active.take().expect("matching active turn");
            turn.record_timing();
            turn.report.error = error;
            self.indicators.finish(turn.report.source);
            if turn.report.stage == Stage::AwaitingWake {
                self.wake_models.changed.notify_one();
            }
            runtime.last = Some(turn.report);
        }
    }

    pub fn clear_chat(&self) -> Result<(), Failure> {
        let mut runtime = self.runtime();
        if runtime.active.is_some() {
            return Err(Failure::Busy);
        }
        runtime.messages.clear();
        Ok(())
    }

    fn push(&self, role: &'static str, text: String) {
        let mut runtime = self.runtime();
        runtime.next_message += 1;
        let id = runtime.next_message;
        runtime.messages.push_back(Message { id, role, text });
        if runtime.messages.len() > 60 {
            runtime.messages.pop_front();
        }
    }

    fn request_settings(&self, id: u64) -> Option<RequestSettings> {
        self.runtime()
            .active
            .as_ref()
            .filter(|turn| turn.id == id)
            .map(|turn| turn.request.clone())
    }

    pub async fn chat(&self, id: u64, text: String) {
        let Some(request) = self.request_settings(id) else {
            return;
        };
        let instructions = if request.system_prompt.is_empty() {
            CHAT_INSTRUCTIONS
        } else {
            &request.system_prompt
        };
        let context: Vec<_> = self
            .runtime()
            .messages
            .iter()
            .rev()
            .take(12)
            .cloned()
            .collect();
        self.push("user", text.clone());
        let provider = match request
            .settings
            .assistant
            .resolve(&self.plugins, Capability::Chat)
        {
            Ok(provider) => provider,
            Err(error) => {
                self.finish(id, Some(error));
                return;
            }
        };
        let error = match provider
            .reply(
                &text,
                instructions,
                context
                    .iter()
                    .rev()
                    .map(|message| (message.role, message.text.as_str())),
            )
            .await
        {
            Ok(answer) => {
                self.push("assistant", answer);
                None
            }
            Err(error) => Some(Provider::failure(error)),
        };
        self.finish(id, error);
    }
}

impl Turn {
    fn record_timing(&mut self) {
        let now = Instant::now();
        self.report.timings.push(Timing {
            stage: self.report.stage,
            ms: now.duration_since(self.started).as_millis() as u64,
        });
        self.started = now;
    }
}

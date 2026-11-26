use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use goodwin_plugin::provider::Capability;

use crate::panel_core::{
    ChatTest, Message, PlaybackMode, Report, VoiceDetails, VoiceModel, VoiceReport, recent_chat,
};
pub use crate::panel_core::{Failure, Source, Stage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::{
    assistant::{PipelineSettings, StageSettings},
    config::{Config, ConfigStore, RequestSettings, Settings},
    plugins::{Plugins, View as PluginView},
    providers::Provider,
    station::{Indicators, Player},
    voice_core::{VoiceProviders, VoiceRequest},
    wake::{PendingModel, WakeModels},
    wake_catalog::Selection,
};

#[cfg(test)]
mod tests;
mod voice;

impl From<&StageSettings> for VoiceModel {
    fn from(settings: &StageSettings) -> Self {
        Self {
            provider: settings.provider.clone(),
            model: settings.selection.model.clone(),
        }
    }
}

impl VoiceDetails {
    fn new(pipeline: &PipelineSettings) -> Self {
        Self {
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            transcription: (&pipeline.transcription).into(),
            chat: (&pipeline.chat).into(),
            speech: (&pipeline.speech).into(),
            transcript: None,
            answer: None,
            playback: None,
            error_stage: None,
        }
    }
}

struct Turn {
    id: u64,
    started: Instant,
    report: Report,
    request: RequestSettings,
    wake_word: Option<String>,
    voice: Option<VoiceDetails>,
    voice_request: Option<VoiceRequest>,
    voice_providers: Option<Result<VoiceProviders<Provider>, (Stage, Failure)>>,
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

#[derive(Default)]
struct Runtime {
    next_id: u64,
    next_message: u64,
    voice_turn: u64,
    voice_history_after: u64,
    active: Option<Turn>,
    last: Option<Report>,
    last_chat_test: Option<Report>,
    last_voice: Option<VoiceReport>,
    last_voice_test: Option<VoiceReport>,
    microphone: bool,
    model: bool,
    wake_model: Option<Selection>,
    wake_model_switching: bool,
    messages: VecDeque<Message>,
    chat_draft: Option<Message>,
}

pub struct App {
    pub dir: PathBuf,
    config: ConfigStore,
    runtime: Mutex<Runtime>,
    microphone_mute: watch::Sender<bool>,
    changes: watch::Sender<()>,
    chat_changes: watch::Sender<()>,
    panel_shutdown: watch::Sender<bool>,
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
    last_chat_test: Option<Report>,
    chat_test_current: bool,
    last_voice: Option<VoiceReport>,
    last_voice_test: Option<VoiceReport>,
    voice_test_current: bool,
    messages: VecDeque<Message>,
    plugins: Vec<PluginView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) chat_draft: Option<Message>,
}

impl Snapshot {
    pub(crate) fn chat_cursor(&self) -> Option<(u64, usize)> {
        self.chat_draft
            .as_ref()
            .map(|draft| (draft.id, draft.text.len()))
    }
}

#[derive(Serialize)]
pub(crate) struct ChatDelta {
    id: u64,
    text: String,
}

impl App {
    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
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
            changes: watch::Sender::default(),
            chat_changes: watch::Sender::default(),
            panel_shutdown: watch::Sender::default(),
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

    pub(crate) fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub(crate) fn chat_changes(&self) -> watch::Receiver<()> {
        self.chat_changes.subscribe()
    }

    pub(crate) fn panel_shutdown(&self) -> watch::Receiver<bool> {
        self.panel_shutdown.subscribe()
    }

    pub(crate) fn stop_panel(&self) {
        self.panel_shutdown.send_replace(true);
    }

    pub(crate) fn chat_delta(&self, cursor: &mut Option<(u64, usize)>) -> Option<ChatDelta> {
        let runtime = self.runtime();
        let draft = runtime.chat_draft.as_ref()?;
        let offset = cursor
            .filter(|(id, _)| *id == draft.id)
            .map_or(0, |(_, offset)| offset);
        let text = draft.text.get(offset..).filter(|text| !text.is_empty())?;
        *cursor = Some((draft.id, draft.text.len()));
        Some(ChatDelta {
            id: draft.id,
            text: text.to_owned(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        let runtime = self.runtime();
        let request = self.config.request_settings();
        let chat_test_current = runtime
            .last_chat_test
            .as_ref()
            .and_then(|report| report.tested_configuration)
            .is_some_and(|tested| {
                tested == self.test_configuration(Source::Chat, &request, &runtime)
            });
        let voice_test_current = runtime
            .last_voice_test
            .as_ref()
            .and_then(|test| {
                test.report
                    .tested_configuration
                    .map(|tested| (test.report.source, tested))
            })
            .is_some_and(|(source, tested)| {
                tested == self.test_configuration(source, &request, &runtime)
            });
        let RequestSettings {
            settings,
            system_prompt,
        } = request;
        Snapshot {
            settings,
            system_prompt,
            microphone: runtime.microphone,
            default_microphone_gain: self.default_microphone_gain,
            model: runtime.model,
            wake_model: runtime.wake_model.clone(),
            active: runtime.active.as_ref().map(|turn| turn.report.clone()),
            last: runtime.last.clone(),
            last_chat_test: runtime.last_chat_test.clone(),
            chat_test_current,
            last_voice: runtime.last_voice.clone(),
            last_voice_test: runtime.last_voice_test.clone(),
            voice_test_current,
            messages: runtime.messages.clone(),
            plugins: self.plugins.catalog(),
            chat_draft: runtime.chat_draft.clone(),
        }
    }

    fn test_configuration(
        &self,
        source: Source,
        request: &RequestSettings,
        runtime: &Runtime,
    ) -> [u8; 32] {
        let settings = &request.settings;
        let pipeline = &settings.assistant.pipeline;
        let input = if source == Source::Chat {
            let providers = self
                .plugins
                .provider_revisions([(&pipeline.chat.provider, Capability::Chat)]);
            goodwin_json::to_vec(&(
                &pipeline.chat,
                settings.languages.reply,
                &request.system_prompt,
                providers,
            ))
        } else {
            let providers = self.plugins.provider_revisions([
                (&pipeline.transcription.provider, Capability::Transcription),
                (&pipeline.chat.provider, Capability::Chat),
                (&pipeline.speech.provider, Capability::Speech),
            ]);
            goodwin_json::to_vec(&(
                settings,
                &request.system_prompt,
                providers,
                runtime.model,
                &runtime.wake_model,
            ))
        };
        Sha256::digest(input.expect("test configuration is serializable")).into()
    }

    pub(crate) fn microphone_mute(&self) -> watch::Receiver<bool> {
        self.microphone_mute.subscribe()
    }

    pub(crate) fn set_microphone_muted(&self, muted: bool) -> goodwin_error::Result<()> {
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
        if !runtime.microphone {
            runtime.microphone = true;
            self.changes.send_replace(());
        }
        true
    }

    pub(crate) fn microphone_disconnected(&self) {
        self.end_capture(&mut self.runtime(), Failure::Microphone);
    }

    fn end_capture(&self, runtime: &mut Runtime, error: Failure) {
        if std::mem::take(&mut runtime.microphone) {
            self.changes.send_replace(());
        }
        if let Some(id) = runtime.active.as_ref().and_then(|turn| {
            matches!(
                turn.report.stage,
                Stage::AwaitingWake | Stage::AwaitingSpeech | Stage::Recording
            )
            .then_some(turn.id)
        }) {
            self.finish_turn(runtime, id, Some(error));
        }
    }

    pub fn set_model(&self, loaded: bool, selection: Option<Selection>) {
        let mut runtime = self.runtime();
        runtime.model = loaded;
        runtime.wake_model = selection;
        self.changes.send_replace(());
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
        runtime.wake_model_switching = true;
        Some(pending)
    }

    pub(crate) fn finish_wake_model_switch(&self, selection: Option<&Selection>) {
        let mut runtime = self.runtime();
        runtime.wake_model_switching = false;
        if let Some(selection) = selection {
            runtime.model = true;
            runtime.wake_model = Some(selection.clone());
            self.changes.send_replace(());
        }
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

    pub(crate) fn cancel_voice(&self, id: u64) -> bool {
        let mut runtime = self.runtime();
        let Some(turn) = runtime.active.as_ref().filter(|turn| turn.id == id) else {
            return false;
        };
        let Some(request) = turn
            .voice_request
            .as_ref()
            .filter(|request| request.id == id)
        else {
            return false;
        };
        request.cancel(Failure::Cancelled);
        if matches!(
            turn.report.stage,
            Stage::AwaitingWake | Stage::AwaitingSpeech | Stage::Recording
        ) {
            self.finish_turn(&mut runtime, id, Some(Failure::Cancelled));
            self.wake_models.changed.notify_one();
        }
        true
    }

    pub fn start(&self, source: Source) -> Result<u64, Failure> {
        self.start_turn(source, false)
    }

    pub(crate) fn start_chat_test(&self) -> Result<u64, Failure> {
        self.start_turn(Source::Chat, true)
    }

    fn start_turn(&self, source: Source, chat_test: bool) -> Result<u64, Failure> {
        self.admit_turn(&mut self.runtime(), source, chat_test)
    }

    fn admit_turn(
        &self,
        runtime: &mut Runtime,
        source: Source,
        chat_test: bool,
    ) -> Result<u64, Failure> {
        let request = self.config.request_settings();
        if source != Source::Chat && *self.microphone_mute.borrow() {
            return Err(Failure::MicrophoneMuted);
        }
        if runtime.active.is_some() {
            return Err(Failure::Busy);
        }
        if matches!(source, Source::Wake | Source::WakeTest) && runtime.wake_model_switching {
            return Err(Failure::Busy);
        }
        if source != Source::Chat && !runtime.microphone {
            return Err(Failure::Microphone);
        }
        if matches!(source, Source::Wake | Source::WakeTest) && !runtime.model {
            return Err(Failure::Model);
        }
        if matches!(source, Source::Wake | Source::FollowUp) && !request.settings.listening {
            return Err(Failure::Busy);
        }
        if source == Source::FollowUp && request.settings.dialogue.timeout().is_none() {
            return Err(Failure::Busy);
        }
        if source != Source::FollowUp {
            runtime.voice_history_after = runtime.next_message;
        }
        runtime.next_id += 1;
        let id = runtime.next_id;
        let stage = if source == Source::Chat {
            Stage::Thinking
        } else {
            crate::voice_core::first_voice_stage(source)
        };
        runtime.active = Some(Turn {
            id,
            started: Instant::now(),
            report: Report {
                request_id: (source != Source::Chat).then_some(id),
                source,
                stage,
                timings: Vec::new(),
                error: None,
                message_id: None,
                chat_test: chat_test.then(|| ChatTest {
                    model: (&request.settings.assistant.pipeline.chat).into(),
                    answered: false,
                }),
                tested_configuration: (chat_test
                    || matches!(source, Source::ManualTest | Source::WakeTest))
                .then(|| self.test_configuration(source, &request, runtime)),
            },
            voice: (source != Source::Chat)
                .then(|| VoiceDetails::new(&request.settings.assistant.pipeline)),
            voice_request: (source != Source::Chat).then(|| VoiceRequest::new(id)),
            voice_providers: (source != Source::Chat).then(|| {
                self.plugins
                    .bind_voice(&request.settings.assistant.pipeline)
            }),
            request,
            wake_word: runtime
                .wake_model
                .as_ref()
                .map(|model| model.wake_word.clone()),
        });
        self.indicators.stage(source, stage);
        self.changes.send_replace(());
        log::info!("Request started id={id} source={source:?}");
        Ok(id)
    }

    pub fn stage(&self, id: u64, stage: Stage) {
        let mut runtime = self.runtime();
        if let Some(turn) = runtime.active.as_mut().filter(|turn| {
            turn.id == id
                && turn
                    .voice_request
                    .as_ref()
                    .is_none_or(|request| request.failure().is_none())
        }) {
            if turn.report.stage == stage {
                return;
            }
            turn.record_timing();
            let wake_test_done =
                turn.report.stage == Stage::AwaitingWake && stage != Stage::AwaitingWake;
            turn.report.stage = stage;
            self.indicators.stage(turn.report.source, stage);
            self.changes.send_replace(());
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
            let error = turn
                .voice_request
                .as_ref()
                .and_then(VoiceRequest::failure)
                .or(error);
            // No request was spoken during this window. Keep the last real report.
            if turn.report.stage == Stage::AwaitingSpeech {
                self.indicators.finish(turn.report.source);
                self.changes.send_replace(());
                return;
            }
            turn.record_timing();
            turn.report.error = error;
            let source = turn.report.source;
            let elapsed_ms: u64 = turn.report.timings.iter().map(|timing| timing.ms).sum();
            if let Some(error) = error {
                let stage = turn
                    .voice
                    .as_ref()
                    .and_then(|details| details.error_stage)
                    .unwrap_or(turn.report.stage);
                let capability = match stage {
                    Stage::Transcribing => Some(Capability::Transcription),
                    Stage::Thinking => Some(Capability::Chat),
                    Stage::Synthesizing | Stage::Playing => Some(Capability::Speech),
                    Stage::AwaitingWake | Stage::AwaitingSpeech | Stage::Recording => None,
                };
                let provider = capability.map_or("-", |capability| {
                    turn.request
                        .settings
                        .assistant
                        .pipeline
                        .get(capability)
                        .provider
                        .as_str()
                });
                log::warn!(
                    "Request failed id={id} source={source:?} stage={stage:?} provider={provider} error={error:?} elapsed_ms={elapsed_ms}"
                );
            } else {
                log::info!("Request completed id={id} source={source:?} elapsed_ms={elapsed_ms}");
            }
            if turn.report.stage == Stage::AwaitingWake {
                self.wake_models.changed.notify_one();
            }
            if let Some(error) = error
                && let Some(id) = turn.report.message_id
                && let Some(message) = runtime
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|message| message.id == id)
            {
                message.error.get_or_insert(error);
            }
            if let Some(mut details) = turn.voice {
                if error.is_some() {
                    details.error_stage.get_or_insert(turn.report.stage);
                }
                let voice = VoiceReport {
                    report: turn.report.clone(),
                    details,
                };
                if matches!(turn.report.source, Source::ManualTest | Source::WakeTest) {
                    runtime.last_voice_test = Some(voice.clone());
                }
                runtime.last_voice = Some(voice);
            }
            if turn.report.chat_test.is_some() {
                runtime.last_chat_test = Some(turn.report.clone());
            }
            let continue_dialogue = error.is_none()
                && turn.report.stage == Stage::Playing
                && matches!(source, Source::Wake | Source::FollowUp);
            runtime.last = Some(turn.report);
            if !continue_dialogue || self.admit_turn(runtime, Source::FollowUp, false).is_err() {
                self.indicators.finish(source);
            }
            self.changes.send_replace(());
        }
    }

    pub fn clear_chat(&self) -> Result<(), Failure> {
        let mut runtime = self.runtime();
        if runtime.active.is_some() {
            return Err(Failure::Busy);
        }
        runtime.messages.clear();
        let Runtime {
            last_voice,
            last_voice_test,
            ..
        } = &mut *runtime;
        for voice in [last_voice, last_voice_test].into_iter().flatten() {
            voice.details.transcript = None;
            voice.details.answer = None;
        }
        self.changes.send_replace(());
        Ok(())
    }

    fn update_voice(&self, id: u64, update: impl FnOnce(&mut VoiceDetails, Stage)) {
        let mut runtime = self.runtime();
        if let Some(turn) = runtime.active.as_mut().filter(|turn| turn.id == id)
            && let Some(details) = &mut turn.voice
        {
            update(details, turn.report.stage);
        }
    }

    fn push(&self, role: &'static str, text: String) {
        self.runtime().push_message(role, text);
        self.changes.send_replace(());
    }

    fn append_chat(&self, id: u64, text: &str) {
        let mut runtime = self.runtime();
        if !runtime.active.as_ref().is_some_and(|turn| turn.id == id) {
            return;
        }
        if let Some(draft) = &mut runtime.chat_draft {
            draft.text.push_str(text);
        } else if !text.trim().is_empty() {
            runtime.chat_draft = Some(runtime.message("assistant", text.trim_start().to_owned()));
        } else {
            return;
        }
        self.chat_changes.send_replace(());
    }

    pub async fn chat(&self, id: u64, text: String) {
        let (request, context) = {
            let mut runtime = self.runtime();
            let Some(turn) = runtime.active.as_ref().filter(|turn| turn.id == id) else {
                return;
            };
            let request = turn.request.clone();
            let context: Vec<_> = if turn.report.chat_test.is_some() {
                Vec::new()
            } else {
                recent_chat(runtime.messages.iter().rev())
            };
            let message_id = runtime.push_message("user", text.clone());
            runtime
                .active
                .as_mut()
                .expect("matching active turn")
                .report
                .message_id = Some(message_id);
            self.changes.send_replace(());
            (request, context)
        };
        let instructions = request
            .settings
            .languages
            .chat_instructions(&request.system_prompt);
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
        let answer = provider
            .reply(
                &text,
                &instructions,
                context,
                Some(&mut |text| self.append_chat(id, text)),
            )
            .await;
        let mut runtime = self.runtime();
        if !runtime.active.as_ref().is_some_and(|turn| turn.id == id) {
            return;
        }
        if let Some(test) = runtime
            .active
            .as_mut()
            .and_then(|turn| turn.report.chat_test.as_mut())
        {
            test.answered = answer.as_ref().is_ok_and(|text| !text.trim().is_empty());
        }
        let draft = runtime.chat_draft.take();
        let error = match (answer, draft) {
            (Ok(text), Some(mut draft)) => {
                draft.text = text;
                runtime.add_message(draft);
                None
            }
            (Ok(text), None) => {
                runtime.push_message("assistant", text);
                None
            }
            (Err(error), Some(mut draft)) => {
                draft.error = Some(Failure::ChatInterrupted);
                if let Some(turn) = &mut runtime.active {
                    turn.report.message_id = Some(draft.id);
                }
                runtime.add_message(draft);
                Some(Provider::failure(error))
            }
            (Err(error), None) => Some(Provider::failure(error)),
        };
        self.finish_turn(&mut runtime, id, error);
    }
}

impl Runtime {
    fn push_message(&mut self, role: &'static str, text: String) -> u64 {
        let message = self.message(role, text);
        let id = message.id;
        self.add_message(message);
        id
    }

    fn message(&mut self, role: &'static str, text: String) -> Message {
        self.next_message += 1;
        Message {
            id: self.next_message,
            role,
            text,
            error: None,
        }
    }

    fn add_message(&mut self, message: Message) {
        self.messages.push_back(message);
        if self.messages.len() > 60 {
            self.messages.pop_front();
        }
    }
}

impl Turn {
    fn record_timing(&mut self) {
        let now = Instant::now();
        self.report.record_timing(now.duration_since(self.started));
        self.started = now;
    }
}

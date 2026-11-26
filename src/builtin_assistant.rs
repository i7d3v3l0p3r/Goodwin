use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use goodwin_error::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::panel_core::{
    ChatTest, Failure, Message, PlaybackMode, Report, Source, Stage, VoiceDetails, VoiceModel,
    VoiceReport, recent_chat,
};
use crate::voice_core::VoiceRequest;

mod provider;
mod settings;
pub(crate) use provider::{HttpTransport, Provider, SPEECH_HZ};
pub(crate) use settings::{ProviderSettings, ProviderUpdate};

#[cfg(feature = "wasm-plugins")]
pub(crate) const BUILTIN_PROVIDER: &str = "builtin";

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct AudioSettings {
    pub listening: bool,
    pub microphone_muted: bool,
    pub sensitivity: f64,
    pub wake_gain: Option<f64>,
    pub transcription_gain: Option<f64>,
    #[serde(default)]
    pub dialogue: crate::voice_core::DialogueSettings,
    #[serde(default)]
    pub languages: crate::voice_core::AssistantLanguages,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            listening: cfg!(feature = "wake-tflite"),
            microphone_muted: false,
            sensitivity: 0.5,
            wake_gain: None,
            transcription_gain: None,
            dialogue: Default::default(),
            languages: Default::default(),
        }
    }
}

impl AudioSettings {
    pub fn validate(self) -> Result<Self> {
        self.dialogue.validate()?;
        ensure!(
            (0.0..=1.0).contains(&self.sensitivity),
            "Invalid wake sensitivity"
        );
        for gain in [self.wake_gain, self.transcription_gain]
            .into_iter()
            .flatten()
        {
            ensure!((0.1..=64.0).contains(&gain), "Invalid microphone gain");
        }
        Ok(self)
    }
}

#[derive(Default)]
pub(super) struct State {
    pub provider: ProviderSettings,
    pub system_prompt: String,
    #[cfg(feature = "wasm-plugins")]
    pub assistant: Option<crate::assistant::AssistantSettings>,
    pub audio: AudioSettings,
    #[cfg(feature = "wake-tflite")]
    pub wake_model: Option<crate::wake_catalog::Selection>,
    active: Option<Request>,
    last: Option<Report>,
    last_voice: Option<VoiceReport>,
    messages: VecDeque<Message>,
    next_message: u64,
    voice_history_after: u64,
    next_request: u64,
}

struct Request {
    phase_start: Instant,
    report: Report,
    voice: Option<VoiceDetails>,
    chat_reply: Option<u64>,
    voice_request: Option<VoiceRequest>,
}

impl Request {
    fn tick(&mut self) {
        self.report.record_timing(self.phase_start.elapsed());
        self.phase_start = Instant::now();
    }
}

impl State {
    pub fn lock(state: &Mutex<Self>) -> MutexGuard<'_, Self> {
        state.lock().expect("assistant state")
    }

    pub fn busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn awaiting_speech(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.report.stage == Stage::AwaitingSpeech)
    }

    pub(crate) fn wake_loaded(&self) -> bool {
        #[cfg(feature = "wake-tflite")]
        {
            self.wake_model.is_some()
        }
        #[cfg(not(feature = "wake-tflite"))]
        {
            false
        }
    }

    pub fn start_chat(&mut self, message: String, test: bool) {
        let id = self.push("user", message);
        self.start(Source::Chat, Stage::Thinking, Some(id), None, None);
        let model = self.voice_model(
            &self.provider.chat_model,
            goodwin_plugin::provider::Capability::Chat,
        );
        if test && let Some(request) = &mut self.active {
            request.report.chat_test = Some(ChatTest {
                model,
                answered: false,
            });
        }
    }

    pub fn chat_context(&self) -> Vec<Message> {
        if self
            .active
            .as_ref()
            .is_some_and(|request| request.report.chat_test.is_some())
        {
            return Vec::new();
        }
        recent_chat(self.messages.iter().rev().skip(1))
    }

    pub fn start_voice(&mut self, source: Source) -> VoiceRequest {
        use goodwin_plugin::provider::Capability;
        if source != Source::FollowUp {
            self.voice_history_after = self.next_message;
        }
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .filter(|elapsed| elapsed.as_secs() >= 1_577_836_800) // 2020-01-01 UTC
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        let details = VoiceDetails {
            started_at_ms,
            transcription: self.voice_model(&self.provider.stt_model, Capability::Transcription),
            chat: self.voice_model(&self.provider.chat_model, Capability::Chat),
            speech: self.voice_model(&self.provider.tts_model, Capability::Speech),
            transcript: None,
            answer: None,
            playback: None,
            error_stage: None,
        };
        let stage = crate::voice_core::first_voice_stage(source);
        self.next_request += 1;
        let request = VoiceRequest::new(self.next_request);
        self.start(source, stage, None, Some(details), Some(request.clone()));
        request
    }

    fn start(
        &mut self,
        source: Source,
        stage: Stage,
        message_id: Option<u64>,
        voice: Option<VoiceDetails>,
        voice_request: Option<VoiceRequest>,
    ) {
        self.active = Some(Request {
            phase_start: Instant::now(),
            report: Report {
                request_id: voice_request.as_ref().map(|request| request.id),
                source,
                stage,
                message_id,
                timings: Vec::new(),
                error: None,
                chat_test: None,
            },
            voice,
            chat_reply: None,
            voice_request,
        });
    }

    pub fn stage(&mut self, stage: Stage) {
        if let Some(active) = &mut self.active
            && active.report.stage != stage
            && active
                .voice_request
                .as_ref()
                .is_none_or(|request| request.failure().is_none())
        {
            active.tick();
            active.report.stage = stage;
        }
    }

    pub fn cancel_voice(&self, id: u64) -> bool {
        let Some(request) = self
            .active
            .as_ref()
            .and_then(|active| active.voice_request.as_ref())
            .filter(|request| request.id == id)
        else {
            return false;
        };
        request.cancel(Failure::Cancelled);
        true
    }

    pub fn mute_capture(&self) {
        if let Some(active) = &self.active
            && matches!(
                active.report.stage,
                Stage::AwaitingWake | Stage::AwaitingSpeech | Stage::Recording
            )
            && let Some(request) = &active.voice_request
        {
            request.cancel(Failure::MicrophoneMuted);
        }
    }

    pub fn finish_voice(&mut self, request: &VoiceRequest, error: Option<Failure>) -> bool {
        let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.report.request_id == Some(request.id))
        else {
            return false;
        };
        if active.report.stage == Stage::AwaitingSpeech {
            self.active = None;
            return false;
        }
        let error = request.failure().or(error);
        let continue_dialogue = error.is_none()
            && active.report.stage == Stage::Playing
            && matches!(
                active.report.source,
                Source::Wake | Source::FollowUp | Source::PushToTalk
            )
            && !self.audio.microphone_muted
            && self.audio.dialogue.timeout().is_some();
        self.finish(error);
        continue_dialogue
    }

    pub fn voice_history(&self) -> Vec<Message> {
        recent_chat(
            self.messages
                .iter()
                .rev()
                .take_while(|message| message.id > self.voice_history_after),
        )
    }

    pub fn transcript(&mut self, text: String) {
        if let Some(voice) = self
            .active
            .as_mut()
            .and_then(|active| active.voice.as_mut())
        {
            voice.transcript = Some(text);
        }
    }

    pub fn voice_answer(&mut self, transcript: &str, answer: &str) {
        if let Some(voice) = self
            .active
            .as_mut()
            .and_then(|active| active.voice.as_mut())
        {
            voice.answer = Some(answer.to_owned());
        }
        self.push("user", transcript.to_owned());
        self.push("assistant", answer.to_owned());
    }

    pub fn playing(&mut self, mode: PlaybackMode) {
        self.stage(Stage::Playing);
        if let Some(voice) = self
            .active
            .as_mut()
            .and_then(|active| active.voice.as_mut())
        {
            voice.playback = Some(mode);
        }
    }

    pub fn finish_chat(&mut self, answer: Result<String>) {
        let partial = self.active.as_ref().and_then(|active| active.chat_reply);
        match answer {
            Ok(answer) => {
                if let Some(test) = self
                    .active
                    .as_mut()
                    .and_then(|request| request.report.chat_test.as_mut())
                {
                    test.answered = true;
                }
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| Some(message.id) == partial)
                {
                    message.text = answer;
                } else {
                    self.push("assistant", answer);
                }
                self.finish(None);
            }
            Err(error) => {
                log::warn!("Chat failed: {error:#}");
                self.messages.retain(|message| Some(message.id) != partial);
                self.finish(Some(Failure::ProviderFailed));
            }
        }
    }

    #[cfg(feature = "wasm-plugins")]
    pub(crate) fn chat_chunk(&mut self, chunk: &str) {
        let Some(active) = &self.active else {
            return;
        };
        if active.report.source != Source::Chat {
            return;
        }
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| Some(message.id) == active.chat_reply)
        {
            message.text.push_str(chunk);
        } else {
            let id = self.push("assistant", chunk.to_owned());
            if let Some(active) = &mut self.active {
                active.chat_reply = Some(id);
            }
        }
    }

    fn finish(&mut self, error: Option<Failure>) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        active.tick();
        active.report.error = error;
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| Some(message.id) == active.report.message_id)
        {
            message.error = error;
        }
        if let Some(mut details) = active.voice {
            details.error_stage = error.map(|_| active.report.stage);
            self.last_voice = Some(VoiceReport {
                report: active.report.clone(),
                details,
            });
        }
        self.last = Some(active.report);
    }

    pub fn clear_chat(&mut self) {
        self.messages.clear();
    }

    fn push(&mut self, role: &'static str, text: String) -> u64 {
        if self.messages.len() == 16 {
            self.messages.pop_front();
        }
        self.next_message += 1;
        self.messages.push_back(Message {
            id: self.next_message,
            role,
            text,
            error: None,
        });
        self.next_message
    }

    pub fn snapshot(&self) -> Value {
        let stage = |model: &str| {
            json!({
                "provider": if cfg!(feature = "wasm-plugins") { "builtin" } else { "openai-compatible" },
                "selection": { "model": if model.is_empty() { None } else { Some(model) } },
            })
        };
        let assistant = || {
            json!({ "engine": "pipeline", "pipeline": {
                "transcription": stage(&self.provider.stt_model), "chat": stage(&self.provider.chat_model),
                "speech": stage(&self.provider.tts_model), "speech_streaming": self.provider.speech_streaming,
            }})
        };
        #[cfg(feature = "wasm-plugins")]
        let assistant = self
            .assistant
            .as_ref()
            .map_or_else(assistant, |settings| json!(settings));
        #[cfg(not(feature = "wasm-plugins"))]
        let assistant = assistant();
        #[cfg(not(feature = "wake-tflite"))]
        let wake_model: Option<Value> = None;
        #[cfg(feature = "wake-tflite")]
        let wake_model = self.wake_model.as_ref();
        json!({
            "settings": {
                "assistant": assistant,
                "listening": self.audio.listening, "microphone_muted": self.audio.microphone_muted,
                "sensitivity": self.audio.sensitivity, "wake_gain": self.audio.wake_gain,
                "transcription_gain": self.audio.transcription_gain,
                "dialogue": self.audio.dialogue,
                "languages": self.audio.languages,
            },
            "provider_ready": self.provider.configured(),
            "system_prompt": self.system_prompt, "microphone": true, "default_microphone_gain": 1,
            "model": self.wake_loaded(), "wake_model": wake_model,
            "active": self.active.as_ref().map(|active| &active.report),
            "last": self.last, "last_voice": self.last_voice,
            "messages": self.messages, "plugins": [],
        })
    }
    fn voice_model(
        &self,
        model: &str,
        capability: goodwin_plugin::provider::Capability,
    ) -> VoiceModel {
        #[cfg(feature = "wasm-plugins")]
        if let Some(stage) = self
            .assistant
            .as_ref()
            .map(|settings| settings.pipeline.get(capability))
            && stage.provider != BUILTIN_PROVIDER
        {
            return VoiceModel {
                provider: stage.provider.clone(),
                model: stage.selection.model.clone(),
            };
        }
        let _ = capability;
        VoiceModel {
            provider: "openai-compatible".into(),
            model: (!model.is_empty()).then(|| model.to_owned()),
        }
    }
}

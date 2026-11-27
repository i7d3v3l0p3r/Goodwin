use goodwin_error::Result;

use crate::{
    panel_core::{Message, recent_chat},
    plugins::VoiceTools,
    providers::{Provider, SpeechStream},
    station::{RecordingGain, playback_failure},
    voice_core::{self, Turn, VoiceProviders, VoiceRequest},
    wav::PcmAudio,
};

use super::{App, Failure, PlaybackMode, Stage};

#[cfg(test)]
mod tests;

impl App {
    pub(crate) async fn voice(&self, id: u64, samples: Vec<i16>, woke: bool, gain: RecordingGain) {
        let Some((request, reply_language)) = self
            .runtime()
            .active
            .as_ref()
            .filter(|turn| turn.id == id)
            .and_then(|turn| {
                Some((
                    turn.voice_request.clone()?,
                    turn.request.settings.languages.reply,
                ))
            })
        else {
            return;
        };
        let error = self
            .voice_answer(&request, Input::Recording(samples, gain), woke)
            .await
            .err();
        let announcement: Option<&[u8]> = match error {
            Some(Failure::ProviderUnavailable | Failure::ProviderFailed) => {
                Some(include_bytes!("../../assets/error-ru.wav"))
            }
            Some(Failure::NoSpeech) if woke => {
                Some(include_bytes!("../../assets/no-speech-ru.wav"))
            }
            _ => None,
        };
        if let Some(russian_announcement) = announcement {
            let wav = if reply_language.as_str() == "ru" {
                russian_announcement
            } else {
                include_bytes!("../../assets/request-error.wav")
            };
            self.update_voice(id, |details, stage| {
                details.error_stage.get_or_insert(stage);
                details.playback = Some(PlaybackMode::Announcement);
            });
            self.stage(id, Stage::Playing);
            let _ = request
                .step(async {
                    self.player
                        .play_speech(wav, &self.indicators)
                        .await
                        .map_err(playback_failure)
                })
                .await;
        }
        self.finish(id, error);
    }

    pub(crate) async fn correct_voice(&self, id: u64, text: String) {
        let request = self
            .runtime()
            .active
            .as_ref()
            .filter(|t| t.id == id)
            .and_then(|t| t.voice_request.clone());
        if let Some(request) = request {
            self.update_voice(id, |details, _| details.transcript = Some(text.clone()));
            let error = self
                .voice_answer(&request, Input::Correction(text), false)
                .await
                .err();
            self.finish(id, error);
        }
    }

    async fn voice_answer(
        &self,
        request: &VoiceRequest,
        input: Input,
        woke: bool,
    ) -> Result<(), Failure> {
        let id = request.id;
        let (providers, system_prompt, languages, speech_streaming, wake_word) = self
            .runtime()
            .active
            .as_mut()
            .filter(|turn| turn.id == id)
            .and_then(|turn| {
                Some((
                    turn.voice_providers.take()?,
                    turn.request.system_prompt.clone(),
                    turn.request.settings.languages,
                    turn.request.settings.assistant.pipeline.speech_streaming,
                    turn.wake_word.take(),
                ))
            })
            .ok_or(Failure::Busy)?;
        request.check()?;
        let providers = providers.map_err(|(stage, error)| {
            self.update_voice(id, |details, _| details.error_stage = Some(stage));
            error
        })?;
        let input = match input {
            Input::Correction(text) => voice_core::VoiceInput::Text(text),
            Input::Recording(mut samples, gain) => {
                gain.prepare(&mut samples);
                voice_core::recording_wav(samples)
                    .map_err(|error| {
                        log::warn!("Could not encode recording id={id}: {error}");
                        Failure::Microphone
                    })?
                    .into()
            }
        };
        voice_core::run(
            request,
            &mut StationTurn {
                app: self,
                id,
                providers,
                system_prompt: &system_prompt,
                languages,
                speech_streaming,
                request,
            },
            input,
            woke.then_some(wake_word.as_deref().unwrap_or("Гудвин")),
        )
        .await
    }
}

enum Input {
    Recording(Vec<i16>, RecordingGain),
    Correction(String),
}

struct StationTurn<'a> {
    request: &'a VoiceRequest,
    app: &'a App,
    id: u64,
    providers: VoiceProviders<Provider>,
    system_prompt: &'a str,
    languages: voice_core::AssistantLanguages,
    speech_streaming: bool,
}

enum Speech {
    Buffered(Vec<u8>),
    Streaming(PcmAudio, SpeechStream),
}

impl Turn for StationTurn<'_> {
    type Speech = Speech;

    fn stage(&mut self, stage: Stage) {
        self.app.stage(self.id, stage);
    }

    async fn transcribe(&mut self, wav: &[u8]) -> Result<String, Failure> {
        let text = self
            .providers
            .transcription
            .transcribe(wav, self.languages.transcription.as_str())
            .await
            .map_err(Provider::failure)?;
        self.app.update_voice(self.id, |details, _| {
            details.transcript = Some(text.clone())
        });
        Ok(text)
    }

    async fn reply(&mut self, text: &str) -> Result<String, Failure> {
        let (voice_turn, history) = {
            let mut runtime = self.app.runtime();
            let history = recent_chat(
                runtime
                    .messages
                    .iter()
                    .rev()
                    .take_while(|message| message.id > runtime.voice_history_after),
            );
            runtime.push_message("user", text.to_owned());
            runtime.voice_turn += 1;
            (runtime.voice_turn, history)
        };
        self.app.changes.send_replace(());
        reply_with_tools(
            &self.providers.chat,
            text,
            &self.languages.voice_instructions(self.system_prompt),
            history,
            self.app.plugins.voice_tools(voice_turn),
            self.request,
        )
        .await
        .map_err(Provider::failure)
    }

    async fn synthesize(&mut self, answer: String) -> Result<Speech, Failure> {
        self.app
            .update_voice(self.id, |details, _| details.answer = Some(answer.clone()));
        self.app.push("assistant", answer.clone());
        if self.speech_streaming
            && let Some(mut stream) = self
                .providers
                .speech
                .stream_speech(&answer)
                .map_err(Provider::failure)?
        {
            let first = stream
                .next()
                .await
                .map_err(Provider::failure)?
                .ok_or(Failure::ProviderFailed)?;
            return Ok(Speech::Streaming(first, stream));
        }
        let wav = self
            .providers
            .speech
            .synthesize_wav(&answer)
            .await
            .map_err(Provider::failure)?;
        Ok(Speech::Buffered(wav))
    }

    async fn play(&mut self, speech: Speech) -> Result<(), Failure> {
        self.app.update_voice(self.id, |details, _| {
            details.playback = Some(match speech {
                Speech::Buffered(_) => PlaybackMode::Buffered,
                Speech::Streaming(..) => PlaybackMode::Streaming,
            });
        });
        self.app.stage(self.id, Stage::Playing);
        match speech {
            Speech::Buffered(wav) => self
                .app
                .player
                .play_speech(&wav, &self.app.indicators)
                .await
                .map_err(playback_failure),
            Speech::Streaming(first, mut stream) => {
                self.app
                    .player
                    .play_stream(first, &mut stream, &self.app.indicators)
                    .await
            }
        }
    }
}

async fn reply_with_tools(
    provider: &Provider,
    text: &str,
    instructions: &str,
    history: Vec<Message>,
    tools: VoiceTools,
    request: &VoiceRequest,
) -> Result<String> {
    let definitions = tools.definitions();
    if definitions.len() == 0 {
        return provider.reply(text, instructions, history, None).await;
    }
    let context = tools.context().await;
    let tools = &tools;
    crate::voice_core::tools::complete(
        instructions,
        text,
        history,
        definitions,
        context,
        provider,
        |call| async move {
            let value = tools.call(&call.name, &call.arguments).await;
            request.record_tool(&call.name, &value);
            value
        },
    )
    .await
}

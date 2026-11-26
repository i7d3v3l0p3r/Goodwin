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
        let Some(request) = self
            .runtime()
            .active
            .as_ref()
            .filter(|turn| turn.id == id)
            .and_then(|turn| turn.voice_request.clone())
        else {
            return;
        };
        let error = self.voice_answer(&request, samples, woke, gain).await.err();
        let announcement: Option<&[u8]> = match error {
            Some(Failure::ProviderUnavailable | Failure::ProviderFailed) => {
                Some(include_bytes!("../../assets/error-ru.wav"))
            }
            Some(Failure::NoSpeech) if woke => {
                Some(include_bytes!("../../assets/no-speech-ru.wav"))
            }
            _ => None,
        };
        if let Some(wav) = announcement {
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

    async fn voice_answer(
        &self,
        request: &VoiceRequest,
        mut samples: Vec<i16>,
        woke: bool,
        gain: RecordingGain,
    ) -> Result<(), Failure> {
        let id = request.id;
        let (providers, system_prompt, speech_streaming, wake_word) = self
            .runtime()
            .active
            .as_mut()
            .filter(|turn| turn.id == id)
            .and_then(|turn| {
                Some((
                    turn.voice_providers.take()?,
                    turn.request.system_prompt.clone(),
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
        gain.prepare(&mut samples);
        let wav = voice_core::recording_wav(samples).map_err(|error| {
            log::warn!("Could not encode recording id={id}: {error}");
            Failure::Microphone
        })?;
        voice_core::run(
            request,
            &mut StationTurn {
                app: self,
                id,
                providers,
                system_prompt: &system_prompt,
                speech_streaming,
            },
            wav,
            woke.then_some(wake_word.as_deref().unwrap_or("Гудвин")),
        )
        .await
    }
}

struct StationTurn<'a> {
    app: &'a App,
    id: u64,
    providers: VoiceProviders<Provider>,
    system_prompt: &'a str,
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
            .transcribe(wav)
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
            self.system_prompt,
            history,
            self.app.plugins.voice_tools(voice_turn),
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
    system_prompt: &str,
    history: Vec<Message>,
    tools: VoiceTools,
) -> Result<String> {
    let voice_instructions = voice_core::instructions(system_prompt);
    let definitions = tools.definitions();
    if definitions.len() == 0 {
        return provider
            .reply(text, &voice_instructions, history, None)
            .await;
    }
    let context = tools.context().await;
    let tools = &tools;
    crate::voice_core::tools::complete(
        &voice_instructions,
        text,
        history,
        definitions,
        context,
        provider,
        |call| async move { tools.call(&call.name, &call.arguments).await },
    )
    .await
}

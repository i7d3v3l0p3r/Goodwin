use goodwin_error::{Context, Result};
use goodwin_plugin::provider::Capability;

use crate::{
    assistant::AssistantSettings,
    plugins::VoiceTools,
    providers::{Provider, SpeechStream},
    station::{RecordingGain, playback_failure},
    voice_core::{self, Turn},
    wav::PcmAudio,
};

use super::{App, Failure, PlaybackMode, Stage};

#[cfg(test)]
mod tests;

impl App {
    pub(crate) async fn voice(&self, id: u64, samples: Vec<i16>, woke: bool, gain: RecordingGain) {
        let error = self.voice_answer(id, samples, woke, gain).await.err();
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
            if let Err(error) = self.player.play_speech(wav, &self.indicators).await {
                log::warn!("Offline announcement failed id={id}: {error:#}");
            }
        }
        self.finish(id, error);
    }

    async fn voice_answer(
        &self,
        id: u64,
        mut samples: Vec<i16>,
        woke: bool,
        gain: RecordingGain,
    ) -> Result<(), Failure> {
        let (request, wake_word) = self
            .runtime()
            .active
            .as_ref()
            .filter(|turn| turn.id == id)
            .map(|turn| (turn.request.clone(), turn.wake_word.clone()))
            .ok_or(Failure::Busy)?;
        let engine = &request.settings.assistant;
        let transcription = self.voice_provider(id, engine, Capability::Transcription)?;
        let chat = self.voice_provider(id, engine, Capability::Chat)?;
        let speech = self.voice_provider(id, engine, Capability::Speech)?;
        gain.prepare(&mut samples);
        let wav = voice_core::recording_wav(samples).map_err(|error| {
            log::warn!("Could not encode recording id={id}: {error}");
            Failure::Microphone
        })?;
        voice_core::run(
            &mut StationTurn {
                app: self,
                id,
                transcription,
                chat,
                speech,
                system_prompt: &request.system_prompt,
                speech_streaming: engine.pipeline.speech_streaming,
            },
            wav,
            woke.then_some(wake_word.as_deref().unwrap_or("Гудвин")),
        )
        .await
    }

    fn voice_provider(
        &self,
        id: u64,
        engine: &AssistantSettings,
        capability: Capability,
    ) -> Result<Provider, Failure> {
        engine.resolve(&self.plugins, capability).inspect_err(|_| {
            self.update_voice(id, |details, _| {
                details.error_stage = Some(match capability {
                    Capability::Transcription => Stage::Transcribing,
                    Capability::Chat => Stage::Thinking,
                    Capability::Speech => Stage::Synthesizing,
                });
            });
        })
    }
}

struct StationTurn<'a> {
    app: &'a App,
    id: u64,
    transcription: Provider,
    chat: Provider,
    speech: Provider,
    system_prompt: &'a str,
    speech_streaming: bool,
}

enum Speech {
    Buffered(Vec<u8>),
    Streaming(PcmAudio, SpeechStream),
}

impl Turn for StationTurn<'_> {
    type Error = Failure;
    type Speech = Speech;

    async fn transcribe(&mut self, wav: &[u8]) -> Result<String, Failure> {
        let text = self
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
        self.app.push("user", text.to_owned());
        self.app.stage(self.id, Stage::Thinking);
        let voice_turn = {
            let mut runtime = self.app.runtime();
            runtime.voice_turn += 1;
            runtime.voice_turn
        };
        reply_with_tools(
            &self.chat,
            text,
            self.system_prompt,
            self.app.plugins.voice_tools(voice_turn),
        )
        .await
        .map_err(Provider::failure)
    }

    async fn synthesize(&mut self, answer: String) -> Result<Speech, Failure> {
        self.app
            .update_voice(self.id, |details, _| details.answer = Some(answer.clone()));
        self.app.push("assistant", answer.clone());
        self.app.stage(self.id, Stage::Synthesizing);
        if self.speech_streaming
            && let Some(mut stream) = self
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
        let audio = self
            .speech
            .synthesize(&answer)
            .await
            .map_err(Provider::failure)?;
        let mut wav = Vec::with_capacity(audio.data.len() + 44);
        audio
            .write_wav(&mut wav)
            .context("Encode synthesized speech")
            .map_err(playback_failure)?;
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

    fn no_speech(&self) -> Failure {
        Failure::NoSpeech
    }
}

async fn reply_with_tools(
    provider: &Provider,
    text: &str,
    system_prompt: &str,
    tools: VoiceTools,
) -> Result<String> {
    let voice_instructions = voice_core::instructions(system_prompt);
    let definitions = tools.definitions();
    if definitions.len() == 0 {
        return provider.reply(text, &voice_instructions, [], None).await;
    }
    let context = tools.context().await;
    let tools = &tools;
    crate::voice_core::tools::complete(
        &voice_instructions,
        text,
        definitions,
        context,
        |request| provider.complete(request),
        |call| async move { tools.call(&call.name, &call.arguments).await },
    )
    .await
}

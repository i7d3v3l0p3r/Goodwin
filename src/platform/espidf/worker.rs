use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use esp_idf_svc::hal::task::block_on;
use goodwin_error::{Context, Result};

use crate::{
    builtin_assistant::{ProviderSettings, State},
    panel_core::{Failure, Message, PlaybackMode, Stage},
    voice_core::{self, RecordingWav, Turn},
};

use super::{audio::Audio, cascade::Cascade, network::Network, panel::Panel};

pub(super) struct Worker {
    jobs: mpsc::SyncSender<Job>,
    playback: mpsc::Receiver<PlaybackRequest>,
}

pub(super) struct RequestContext {
    provider: ProviderSettings,
    prompt: String,
    #[cfg(feature = "wasm-plugins")]
    assistant: Option<crate::assistant::AssistantSettings>,
}

impl RequestContext {
    pub(super) fn new(state: &State) -> Self {
        Self {
            provider: state.provider.clone(),
            prompt: state.system_prompt.clone(),
            #[cfg(feature = "wasm-plugins")]
            assistant: state.assistant.clone(),
        }
    }

    fn provider<'a>(&'a self, _panel: &'a Panel) -> Cascade<'a> {
        let provider = Cascade::new(&self.provider, &self.prompt);
        #[cfg(feature = "wasm-plugins")]
        let provider = provider.with_plugins(
            _panel.plugins.get().map(Arc::as_ref),
            self.assistant.as_ref(),
        );
        provider
    }
}

enum Job {
    Chat(RequestContext, String, Vec<Message>),
    Voice(RequestContext, RecordingWav, Option<String>),
}

enum PlaybackRequest {
    Buffered(Vec<u8>, mpsc::Sender<Result<()>>),
    Streaming(mpsc::Receiver<Chunk>, mpsc::Sender<Result<()>>),
}

enum Chunk {
    Pcm(Vec<u8>),
    End,
}

impl Worker {
    pub(super) fn start(panel: Arc<Panel>) -> Result<Self> {
        let (jobs, incoming) = mpsc::sync_channel(1);
        let (output, playback) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("goodwin-cloud".into())
            .stack_size(36 * 1024)
            .spawn(move || {
                while let Ok(job) = incoming.recv() {
                    process(job, &panel, &output);
                }
            })
            .context("Start Goodwin provider task")?;
        Ok(Self { jobs, playback })
    }

    pub(super) fn chat(&self, message: String, panel: &Panel, network: &mut Network<'_>) {
        let (context, history) = {
            let state = State::lock(&panel.state);
            (RequestContext::new(&state), state.chat_context())
        };
        let result = network
            .ready(context.provider(panel).https())
            .and_then(|()| {
                self.jobs
                    .try_send(Job::Chat(context, message, history))
                    .map_err(|_| goodwin_error::report!("Provider task is unavailable"))
            });
        if let Err(error) = result {
            State::lock(&panel.state).finish_chat(Err(error));
        }
    }

    pub(super) fn voice(
        &self,
        context: RequestContext,
        wav: RecordingWav,
        wake_word: Option<String>,
        panel: &Panel,
        network: &mut Network<'_>,
    ) -> Result<(), Failure> {
        network
            .ready(context.provider(panel).https())
            .map_err(provider_failure)?;
        self.jobs
            .try_send(Job::Voice(context, wav, wake_word))
            .map_err(|_| Failure::ProviderUnavailable)
    }

    /// The device loop is the only owner of the I²S driver.
    pub(super) fn play_ready(&self, audio: &mut Audio<'_>, panel: &Panel) {
        match self.playback.try_recv() {
            Ok(PlaybackRequest::Buffered(pcm, done)) => {
                State::lock(&panel.state).playing(PlaybackMode::Buffered);
                let _ = done.send(audio.play(&pcm));
            }
            Ok(PlaybackRequest::Streaming(chunks, done)) => {
                let result = play_stream(audio, &panel.state, &chunks);
                let _ = done.send(result);
            }
            Err(_) => {}
        }
    }
}

fn play_stream(
    audio: &mut Audio<'_>,
    state: &Mutex<State>,
    chunks: &mpsc::Receiver<Chunk>,
) -> Result<()> {
    let mut playback = audio.playback();
    let mut started = false;
    loop {
        match chunks.recv_timeout(Duration::from_secs(20)) {
            Ok(Chunk::Pcm(pcm)) => {
                if !started {
                    State::lock(state).playing(PlaybackMode::Streaming);
                }
                playback.push(&pcm)?;
                if !started {
                    started = true;
                    log::info!("Streamed playback started");
                }
            }
            Ok(Chunk::End) => return playback.finish(),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(error) => return Err(error).context("Wait for speech audio"),
        }
    }
}

fn process(job: Job, panel: &Panel, output: &mpsc::SyncSender<PlaybackRequest>) {
    let state = &panel.state;
    match job {
        Job::Chat(context, message, history) => {
            let answer = context.provider(panel).chat(&message, &history, |_chunk| {
                #[cfg(feature = "wasm-plugins")]
                State::lock(state).chat_chunk(_chunk);
            });
            State::lock(state).finish_chat(answer);
        }
        Job::Voice(context, wav, wake_word) => {
            let provider = context.provider(panel);
            let streaming = provider.streaming();
            let result = block_on(voice_core::run(
                &mut EspTurn {
                    provider,
                    output,
                    state,
                    transcript: String::new(),
                    streaming,
                    #[cfg(feature = "wasm-plugins")]
                    tools: panel.plugins.get().map(|plugins| plugins.voice_tools()),
                },
                wav,
                wake_word.as_deref(),
            ));
            State::lock(state).finish(result.err());
            match result {
                Ok(()) => log::info!("Voice turn complete"),
                Err(error) => log::warn!("Voice turn failed: {error:?}"),
            }
        }
    }
}

struct EspTurn<'a> {
    provider: Cascade<'a>,
    output: &'a mpsc::SyncSender<PlaybackRequest>,
    state: &'a Mutex<State>,
    transcript: String,
    streaming: bool,
    #[cfg(feature = "wasm-plugins")]
    tools: Option<super::plugins::VoiceTools<'a>>,
}

enum Speech {
    Buffered(Vec<u8>),
    Streaming(String),
}

impl Turn for EspTurn<'_> {
    type Error = Failure;
    type Speech = Speech;

    async fn transcribe(&mut self, wav: &[u8]) -> Result<String, Failure> {
        let text = self.provider.transcribe(wav).map_err(provider_failure)?;
        State::lock(self.state).transcript(text.clone());
        Ok(text)
    }

    async fn reply(&mut self, text: &str) -> Result<String, Failure> {
        State::lock(self.state).stage(Stage::Thinking);
        self.transcript = text.to_owned();
        #[cfg(feature = "wasm-plugins")]
        if let Some(tools) = &self.tools
            && tools.definitions().len() != 0
        {
            return voice_core::tools::complete(
                &self.provider.voice_instructions(),
                text,
                tools.definitions(),
                tools.context(),
                |request| std::future::ready(self.provider.complete_tools(request)),
                |call| std::future::ready(tools.call(&call.name, &call.arguments)),
            )
            .await
            .map_err(provider_failure);
        }
        self.provider.reply(text).map_err(provider_failure)
    }

    async fn synthesize(&mut self, answer: String) -> Result<Speech, Failure> {
        {
            let mut state = State::lock(self.state);
            state.voice_answer(&self.transcript, &answer);
            state.stage(Stage::Synthesizing);
        }
        if self.streaming {
            Ok(Speech::Streaming(answer))
        } else {
            self.provider
                .synthesize(&answer)
                .map(Speech::Buffered)
                .map_err(provider_failure)
        }
    }

    async fn play(&mut self, speech: Speech) -> Result<(), Failure> {
        let (done, finished) = mpsc::channel();
        match speech {
            Speech::Buffered(pcm) => self
                .output
                .send(PlaybackRequest::Buffered(pcm, done))
                .map_err(|_| Failure::Playback)?,
            Speech::Streaming(text) => {
                let (chunks, incoming) = mpsc::sync_channel(2);
                let mut pending = Some(PlaybackRequest::Streaming(incoming, done));
                let streamed = self.provider.stream_speech(&text, |pcm| {
                    if pcm.is_empty() {
                        return Ok(());
                    }
                    if let Some(request) = pending.take() {
                        self.output
                            .send(request)
                            .map_err(|_| goodwin_error::report!("Audio task stopped"))?;
                    }
                    chunks
                        .send(Chunk::Pcm(pcm.to_vec()))
                        .map_err(|_| goodwin_error::report!("Audio task stopped"))?;
                    Ok(())
                });
                if pending.is_some() {
                    streamed.map_err(provider_failure)?;
                    return Err(Failure::Playback);
                }
                if streamed.is_ok() {
                    let _ = chunks.send(Chunk::End);
                }
                drop(chunks);
                finished
                    .recv()
                    .map_err(|_| Failure::Playback)?
                    .map_err(playback_failure)?;
                streamed.map_err(provider_failure)?;
                return Ok(());
            }
        }
        finished
            .recv()
            .map_err(|_| Failure::Playback)?
            .map_err(playback_failure)
    }

    fn no_speech(&self) -> Failure {
        Failure::NoSpeech
    }
}

fn playback_failure(error: goodwin_error::Error) -> Failure {
    log::warn!("Play I2S audio: {error:#}");
    Failure::Playback
}

fn provider_failure(error: goodwin_error::Error) -> Failure {
    log::warn!("Voice provider: {error:#}");
    Failure::ProviderFailed
}

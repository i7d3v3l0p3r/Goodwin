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
    voice_core::{self, RecordingGain, RecordingWav, Turn, VoiceRequest},
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

pub(super) struct VoiceJob {
    pub(super) request: VoiceRequest,
    context: RequestContext,
    #[cfg(feature = "wasm-plugins")]
    bindings: Option<voice_core::VoiceProviders<Option<super::plugins::ProviderBinding>>>,
    pub(super) gain: RecordingGain,
    pub(super) wake_word: Option<String>,
    #[cfg(feature = "wake-tflite")]
    pub(super) audio: crate::builtin_assistant::AudioSettings,
}

impl VoiceJob {
    pub(super) fn admit(
        state: &mut State,
        panel: &Panel,
        source: crate::panel_core::Source,
    ) -> Result<Self, Failure> {
        if !panel.voice_ready(state) {
            return Err(Failure::ProviderUnavailable);
        }
        #[cfg(feature = "wasm-plugins")]
        let bindings = state
            .assistant
            .as_ref()
            .zip(panel.plugins.get())
            .map(|(settings, plugins)| {
                plugins
                    .bind_voice(&settings.pipeline)
                    .map_err(|_| Failure::ProviderUnavailable)
            })
            .transpose()?;
        Ok(Self {
            request: state.start_voice(source),
            context: RequestContext::new(state),
            #[cfg(feature = "wasm-plugins")]
            bindings,
            gain: RecordingGain::new(state.audio.transcription_gain.map(|gain| gain as f32), 1.0),
            #[cfg(feature = "wake-tflite")]
            wake_word: state
                .wake_model
                .as_ref()
                .filter(|_| {
                    matches!(
                        source,
                        crate::panel_core::Source::Wake | crate::panel_core::Source::WakeTest
                    )
                })
                .map(|selection| selection.wake_word.clone()),
            #[cfg(not(feature = "wake-tflite"))]
            wake_word: None,
            #[cfg(feature = "wake-tflite")]
            audio: state.audio,
        })
    }

    fn provider<'a>(&'a self, panel: &'a Panel) -> Cascade<'a> {
        let provider = self.context.provider(panel, Some(&self.request));
        #[cfg(feature = "wasm-plugins")]
        let provider = match &self.bindings {
            Some(bindings) => provider.bind_voice(bindings, &self.request),
            None => provider,
        };
        provider
    }
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

    fn provider<'a>(&'a self, _panel: &'a Panel, request: Option<&'a VoiceRequest>) -> Cascade<'a> {
        let provider = Cascade::new(&self.provider, &self.prompt, request);
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
    Voice(VoiceJob, RecordingWav),
}

enum PlaybackRequest {
    Buffered(VoiceRequest, Vec<u8>, mpsc::Sender<Result<()>>),
    Streaming(
        VoiceRequest,
        mpsc::Receiver<Chunk>,
        mpsc::Sender<Result<()>>,
    ),
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
            .ready(context.provider(panel, None).https(), || false)
            .and_then(|()| {
                crate::diagnostics::Queue::Provider
                    .send(&self.jobs, Job::Chat(context, message, history))
                    .map_err(|_| goodwin_error::report!("Provider task is unavailable"))
            });
        if let Err(error) = result {
            State::lock(&panel.state).finish_chat(Err(error));
        }
    }

    pub(super) fn voice(
        &self,
        job: VoiceJob,
        wav: RecordingWav,
        panel: &Panel,
        network: &mut Network<'_>,
    ) -> Result<(), Failure> {
        job.request.check()?;
        let ready = network.ready(job.provider(panel).https(), || {
            job.request.failure().is_some()
        });
        job.request.check()?;
        ready.map_err(provider_failure)?;
        crate::diagnostics::Queue::Provider
            .send(&self.jobs, Job::Voice(job, wav))
            .map_err(|_| Failure::ProviderUnavailable)
    }

    /// The device loop is the only owner of the I²S driver.
    pub(super) fn play_ready(&self, audio: &mut Audio<'_>, panel: &Panel) {
        match self.playback.try_recv() {
            Ok(PlaybackRequest::Buffered(request, pcm, done)) => {
                let result = (|| {
                    if request.failure().is_some() {
                        return Ok(());
                    }
                    State::lock(&panel.state).playing(PlaybackMode::Buffered);
                    let mut playback = audio.playback(Some(&request));
                    playback.push(&pcm)?;
                    playback.finish()
                })();
                let _ = done.send(result);
            }
            Ok(PlaybackRequest::Streaming(request, chunks, done)) => {
                let result = play_stream(audio, &panel.state, &chunks, &request);
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
    request: &VoiceRequest,
) -> Result<()> {
    let mut playback = audio.playback(Some(request));
    let mut started = false;
    let mut last_chunk = std::time::Instant::now();
    loop {
        if request.failure().is_some() {
            return Ok(());
        }
        match chunks.recv_timeout(Duration::from_millis(50)) {
            Ok(Chunk::Pcm(pcm)) => {
                last_chunk = std::time::Instant::now();
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
            Err(mpsc::RecvTimeoutError::Timeout)
                if last_chunk.elapsed() < Duration::from_secs(20) => {}
            Err(error) => return Err(error).context("Wait for speech audio"),
        }
    }
}

fn process(job: Job, panel: &Panel, output: &mpsc::SyncSender<PlaybackRequest>) {
    let state = &panel.state;
    match job {
        Job::Chat(context, message, history) => {
            let answer = context
                .provider(panel, None)
                .chat(&message, &history, |_chunk| {
                    #[cfg(feature = "wasm-plugins")]
                    State::lock(state).chat_chunk(_chunk);
                });
            State::lock(state).finish_chat(answer);
        }
        Job::Voice(job, wav) => {
            let provider = job.provider(panel);
            let streaming = provider.streaming();
            let result = block_on(voice_core::run(
                &job.request,
                &mut EspTurn {
                    request: &job.request,
                    provider,
                    output,
                    state,
                    transcript: String::new(),
                    streaming,
                    #[cfg(feature = "wasm-plugins")]
                    tools: panel
                        .plugins
                        .get()
                        .map(|plugins| plugins.voice_tools(&job.request)),
                },
                wav,
                job.wake_word.as_deref(),
            ));
            State::lock(state).finish_voice(&job.request, result.err());
            match result {
                Ok(()) => log::info!("Voice turn complete"),
                Err(Failure::Cancelled) => log::info!("Voice turn cancelled"),
                Err(error) => log::warn!("Voice turn failed: {error:?}"),
            }
        }
    }
}

struct EspTurn<'a> {
    request: &'a VoiceRequest,
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
    type Speech = Speech;

    fn stage(&mut self, stage: Stage) {
        State::lock(self.state).stage(stage);
    }

    async fn transcribe(&mut self, wav: &[u8]) -> Result<String, Failure> {
        let result = self.provider.transcribe(wav);
        self.request.check()?;
        let text = result.map_err(provider_failure)?;
        State::lock(self.state).transcript(text.clone());
        Ok(text)
    }

    async fn reply(&mut self, text: &str) -> Result<String, Failure> {
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
                .send(PlaybackRequest::Buffered(self.request.clone(), pcm, done))
                .map_err(|_| Failure::Playback)?,
            Speech::Streaming(text) => {
                let (chunks, incoming) = mpsc::sync_channel(2);
                let mut pending = Some(PlaybackRequest::Streaming(
                    self.request.clone(),
                    incoming,
                    done,
                ));
                let streamed = self.provider.stream_speech(&text, |pcm| {
                    goodwin_error::ensure!(
                        self.request.failure().is_none(),
                        "Voice request cancelled"
                    );
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
}

fn playback_failure(error: goodwin_error::Error) -> Failure {
    log::warn!("Play I2S audio: {error:#}");
    Failure::Playback
}

fn provider_failure(error: goodwin_error::Error) -> Failure {
    log::warn!("Voice provider: {error:#}");
    Failure::ProviderFailed
}

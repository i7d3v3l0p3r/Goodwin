pub(crate) mod audio;
mod auth;
mod cascade;
mod firmware;
mod http_client;
mod journal;
#[cfg(feature = "wasm-plugins")]
mod memory;
mod network;
mod panel;
#[cfg(feature = "wasm-plugins")]
pub(crate) mod plugins;
mod settings;
#[cfg(any(feature = "wasm-plugins", feature = "wake-tflite"))]
mod storage;
#[cfg(feature = "wake-tflite")]
mod wake;
#[cfg(feature = "wake-tflite")]
mod wake_models;

use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

use anyhow::Result;
use esp_idf_svc::{
    hal::{delay::FreeRtos, peripherals::Peripherals, task::block_on},
    nvs::EspDefaultNvsPartition,
    sys,
};

use crate::{
    builtin_assistant::State,
    model,
    panel_core::{Failure, PlaybackMode, Source, Stage},
    voice_core::{self, CAPTURE_HZ, RecordingGain, Turn},
};

use self::{
    audio::{Audio, Recording},
    cascade::Cascade,
    network::{Network, WifiCredentials, WifiStatus},
    panel::{Command, Panel},
    settings::Store,
};

pub(crate) fn run() -> Result<()> {
    sys::link_patches();
    journal::init()?;
    let result = run_device();
    if result.is_err()
        && let Err(error) = firmware::verify_boot(false)
    {
        log::error!("Check firmware rollback: {error:#}");
    }
    result
}

fn run_device() -> Result<()> {
    log::info!(
        "Capabilities {}",
        serde_json::to_string(&model::capabilities())?
    );
    // SAFETY: ESP-IDF initialized the external RAM before starting the Rust application.
    log::info!("PSRAM {} bytes", unsafe { sys::esp_psram_get_size() });

    let peripherals = Peripherals::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let store = Store::open(nvs.clone())?;
    let mut wifi = store.wifi().unwrap_or_else(|error| {
        log::warn!("Read Wi-Fi settings: {error:#}");
        None
    });
    let mut audio = model::audio(peripherals.i2s0, peripherals.pins)?;
    let mut network = Network::new(peripherals.modem, nvs)?;
    #[cfg(any(feature = "wasm-plugins", feature = "wake-tflite"))]
    let storage_ready = match storage::mount() {
        Ok(()) => true,
        Err(error) => {
            log::warn!("Add-on storage unavailable: {error:#}");
            false
        }
    };
    #[cfg(feature = "wake-tflite")]
    let mut detector = match load_detector(&store, storage_ready) {
        Ok(detector) => {
            log::info!("Wake model loaded: {}", detector.selection.id);
            Some(detector)
        }
        Err(error) => {
            log::error!("Wake model unavailable: {error:#}");
            None
        }
    };
    #[cfg(feature = "wake-tflite")]
    let wake_loaded = detector.is_some();
    #[cfg(not(feature = "wake-tflite"))]
    let wake_loaded = false;
    let (sender, command) = mpsc::sync_channel(1);
    let panel = Arc::new(Panel::new(
        store,
        sender,
        #[cfg(feature = "wake-tflite")]
        detector.as_ref().map(|detector| detector.selection.clone()),
        #[cfg(feature = "wake-tflite")]
        storage_ready,
    )?);
    let _server = panel::serve(panel.clone())?;
    #[cfg(feature = "wasm-plugins")]
    if storage_ready {
        match plugins::Plugins::start() {
            Ok(plugins) => {
                #[cfg(feature = "wake-tflite")]
                if let Some(detector) = &mut detector {
                    detector.observe_with(Some(plugins.clone()));
                }
                let _ = panel.plugins.set(plugins);
            }
            Err(error) => log::warn!("Plugin storage unavailable: {error:#}"),
        }
    }
    let status = match wifi.as_ref() {
        Some(credentials) => restore_wifi(&mut network, Some(credentials), None)?,
        None => network.setup(None)?,
    };
    *panel.wifi.lock().expect("Wi-Fi status") = status;
    let healthy = !cfg!(feature = "wake-tflite") || wake_loaded;
    #[cfg(feature = "wasm-plugins")]
    let healthy = healthy && (panel.plugins.get().is_some() || !storage::has_partition());
    firmware::verify_boot(healthy)?;
    let mut retry_at = Instant::now() + Duration::from_secs(5);
    let mut failures = 0;
    #[cfg(feature = "wake-tflite")]
    let mut wake_test = None;

    log::info!("Ready for push-to-talk");
    loop {
        if panel.reboot.load(Ordering::Acquire) {
            FreeRtos::delay_ms(1500);
            esp_idf_svc::hal::reset::restart();
        }
        if panel.firmware_busy() {
            #[cfg(feature = "wake-tflite")]
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            let _ = audio.set_receiving(false);
            FreeRtos::delay_ms(20);
            continue;
        }
        #[cfg(feature = "wasm-plugins")]
        if let Some(plugins) = panel.plugins.get() {
            let signals = plugins.take_signals();
            if signals != 0 {
                #[cfg(feature = "wake-tflite")]
                if let Some(detector) = &mut detector {
                    detector.reset();
                }
                let _ = audio.set_receiving(false);
                for _ in 0..signals {
                    if let Err(error) = audio.play_timer_signal() {
                        log::warn!("Play plugin timer signal: {error:#}");
                        break;
                    }
                }
            }
        }
        if let Ok(command) = command.try_recv() {
            #[cfg(feature = "wake-tflite")]
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            if let Err(error) = audio.set_receiving(false) {
                log::warn!("Stop idle audio capture: {error:#}");
            }
            match command {
                Command::Chat(message) => {
                    #[cfg(feature = "wasm-plugins")]
                    let assistant;
                    let (settings, history, prompt) = {
                        let state = State::lock(&panel.state);
                        #[cfg(feature = "wasm-plugins")]
                        {
                            assistant = state.assistant.clone();
                        }
                        (
                            state.provider.clone(),
                            state.chat_context(),
                            state.system_prompt.clone(),
                        )
                    };
                    let provider = Cascade::new(&settings, &prompt);
                    #[cfg(feature = "wasm-plugins")]
                    let provider = provider
                        .with_plugins(panel.plugins.get().map(Arc::as_ref), assistant.as_ref());
                    let answer = network.ready(provider.https()).and_then(|()| {
                        provider.chat(&message, &history, |chunk| {
                            #[cfg(feature = "wasm-plugins")]
                            State::lock(&panel.state).chat_chunk(chunk);
                            let _ = chunk;
                        })
                    });
                    State::lock(&panel.state).finish_chat(answer);
                }
                Command::VoiceTest => {
                    finish_voice(&mut audio, &mut network, Recording::ManualTest, &panel);
                }
                #[cfg(feature = "wake-tflite")]
                Command::WakeTest => {
                    wake_test = Some(Instant::now() + Duration::from_secs(10));
                }
                #[cfg(feature = "wake-tflite")]
                Command::UseWakeModel(candidate, reply) => {
                    let result = (|| {
                        let prepared = wake::Detector::prepare(candidate)?;
                        wake_models::save_asset(&prepared.0, &prepared.1)?;
                        let mut replacement = wake::Detector::new(prepared)?;
                        #[cfg(feature = "wasm-plugins")]
                        replacement.observe_with(panel.plugins.get().cloned());
                        let selection = replacement.selection.clone();
                        panel.save_wake_selection(&selection)?;
                        detector = Some(replacement);
                        State::lock(&panel.state).wake_model = Some(selection.clone());
                        wake_models::remove_unused(&selection);
                        log::info!("Wake model loaded: {}", selection.id);
                        Ok(selection)
                    })();
                    panel.finish_wake_model_change();
                    let _ = reply.send(result);
                }
                Command::Wifi(next) => {
                    // Let the HTTP server send its acknowledgement before changing IP.
                    FreeRtos::delay_ms(500);
                    let status = match network.join(&next) {
                        Ok(ip) if panel.save_wifi(&next).is_ok() => {
                            let status = connected(&next, ip, None);
                            wifi = Some(next);
                            status
                        }
                        Ok(_) => restore_wifi(&mut network, wifi.as_ref(), Some("save_failed"))?,
                        Err(error) => {
                            log::warn!("New Wi-Fi network: {error:#}");
                            restore_wifi(&mut network, wifi.as_ref(), Some("wifi_connect_failed"))?
                        }
                    };
                    *panel.wifi.lock().expect("Wi-Fi status") = status;
                    failures = 0;
                    retry_at = Instant::now() + Duration::from_secs(5);
                }
            }
        }
        if let Some(credentials) = wifi.as_ref()
            && !network.setup_active()
            && Instant::now() >= retry_at
        {
            match network.connect() {
                Ok(ip) => {
                    #[cfg(all(
                        any(feature = "wasm-plugins", feature = "wake-tflite"),
                        not(feature = "esp-emulator")
                    ))]
                    if storage_ready {
                        if let Err(error) = network.start_clock() {
                            log::warn!("Start clock synchronization: {error:#}");
                        }
                        network.poll_clock();
                    }
                    let mut status = panel.wifi.lock().expect("Wi-Fi status");
                    if !matches!(*status, WifiStatus::Connected { address, .. } if address == ip) {
                        *status = connected(credentials, ip, None);
                    }
                    failures = 0;
                }
                Err(error) => {
                    log::warn!("Waiting for Wi-Fi: {error:#}");
                    failures += 1;
                    if failures == 3 {
                        let status = network.setup(Some("wifi_connect_failed"))?;
                        *panel.wifi.lock().expect("Wi-Fi status") = status;
                    }
                }
            }
            retry_at = Instant::now() + Duration::from_secs(5);
        }
        if audio.button_pressed()
            && panel.wifi_connected()
            && start_voice(&panel, Source::PushToTalk)
        {
            #[cfg(feature = "wake-tflite")]
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            finish_voice(&mut audio, &mut network, Recording::Button, &panel);
        }
        #[cfg(feature = "wake-tflite")]
        if let Some(detector) = &mut detector {
            if let Some(deadline) = wake_test {
                let mut state = State::lock(&panel.state);
                let failure = if state.audio.microphone_muted {
                    Some(Failure::MicrophoneMuted)
                } else if Instant::now() >= deadline {
                    Some(Failure::NoWake)
                } else {
                    None
                };
                if failure.is_some() {
                    state.finish(failure);
                    wake_test = None;
                }
            }
            let (enabled, profile) = {
                let state = State::lock(&panel.state);
                (
                    (state.audio.listening || wake_test.is_some())
                        && !state.audio.microphone_muted
                        && (!state.busy() || wake_test.is_some())
                        && panel.voice_ready(&state)
                        && panel.wifi_connected(),
                    state.audio,
                )
            };
            if enabled {
                match detector.listen(
                    &mut audio,
                    profile.wake_gain.unwrap_or(1.0) as f32,
                    profile.sensitivity as f32,
                ) {
                    Ok(Some(recording)) if wake_test.take().is_some() => {
                        State::lock(&panel.state).stage(Stage::Recording);
                        finish_voice(&mut audio, &mut network, recording, &panel);
                        continue;
                    }
                    Ok(Some(recording)) if start_voice(&panel, Source::Wake) => {
                        finish_voice(&mut audio, &mut network, recording, &panel);
                        continue;
                    }
                    Ok(_) => continue,
                    Err(error) => {
                        log::warn!("Wake capture: {error:#}");
                        if wake_test.take().is_some() {
                            State::lock(&panel.state).finish(Some(Failure::Microphone));
                        }
                    }
                }
            }
            detector.reset();
            if let Err(error) = audio.set_receiving(false) {
                log::warn!("Stop wake capture: {error:#}");
            }
        }
        FreeRtos::delay_ms(20);
    }
}

fn connected(
    credentials: &WifiCredentials,
    address: Ipv4Addr,
    error: Option<&'static str>,
) -> WifiStatus {
    log::info!("Goodwin panel http://{address}/");
    WifiStatus::Connected {
        ssid: credentials.ssid.clone(),
        address,
        error,
    }
}

fn restore_wifi(
    network: &mut Network<'_>,
    saved: Option<&WifiCredentials>,
    error: Option<&'static str>,
) -> Result<WifiStatus> {
    let mut reason = error;
    if let Some(credentials) = saved {
        match network.join(credentials) {
            Ok(ip) => return Ok(connected(credentials, ip, error)),
            Err(error) => {
                log::warn!("Saved Wi-Fi network: {error:#}");
                reason = Some("wifi_connect_failed");
            }
        }
    }
    network.setup(reason)
}

fn start_voice(panel: &Panel, source: Source) -> bool {
    let mut state = State::lock(&panel.state);
    if state.busy()
        || panel.firmware_busy()
        || panel.wake_model_busy()
        || state.audio.microphone_muted
        || !panel.voice_ready(&state)
        || (source == Source::Wake && !state.audio.listening)
    {
        return false;
    }
    state.start_voice(source);
    true
}

fn finish_voice(audio: &mut Audio<'_>, network: &mut Network<'_>, mode: Recording, panel: &Panel) {
    let state = &panel.state;
    #[cfg(feature = "wasm-plugins")]
    let assistant;
    let (settings, gain, prompt) = {
        let state = State::lock(state);
        #[cfg(feature = "wasm-plugins")]
        {
            assistant = state.assistant.clone();
        }
        (
            state.provider.clone(),
            RecordingGain::new(state.audio.transcription_gain.map(|gain| gain as f32), 1.0),
            state.system_prompt.clone(),
        )
    };
    let provider = Cascade::new(&settings, &prompt);
    #[cfg(feature = "wasm-plugins")]
    let provider = provider.with_plugins(panel.plugins.get().map(Arc::as_ref), assistant.as_ref());
    let result = voice_request(audio, network, provider, mode, gain, panel);
    State::lock(state).finish(result.err());
    match result {
        Ok(()) => log::info!("Voice turn complete"),
        Err(error) => log::warn!("Voice turn failed: {error:?}"),
    }
}

fn voice_request(
    audio: &mut Audio<'_>,
    network: &mut Network<'_>,
    provider: Cascade<'_>,
    mode: Recording,
    gain: RecordingGain,
    panel: &Panel,
) -> Result<(), Failure> {
    let state = &panel.state;
    #[cfg(feature = "wasm-plugins")]
    let tools = panel.plugins.get().map(|plugins| plugins.voice_tools());
    if !panel.voice_ready(&State::lock(state)) {
        return Err(Failure::ProviderUnavailable);
    }
    let source = mode.source();
    #[cfg(feature = "wake-tflite")]
    let source = State::lock(state).source().unwrap_or(source);
    let muted = || State::lock(state).audio.microphone_muted;
    let mut samples = audio
        .record(mode, gain.input_gain(), muted)
        .map_err(|error| {
            log::warn!("Record I2S audio: {error:#}");
            Failure::Microphone
        })?;
    if muted() {
        return Err(Failure::MicrophoneMuted);
    }
    if samples.len() < CAPTURE_HZ as usize / 5 {
        return Err(Failure::NoSpeech);
    }
    log::info!("Recorded {} samples", samples.len());
    gain.prepare(&mut samples);
    let wav = voice_core::recording_wav(samples).map_err(|error| {
        log::warn!("Encode recording: {error}");
        Failure::Microphone
    })?;
    State::lock(state).stage(Stage::Transcribing);
    network.ready(provider.https()).map_err(provider_failure)?;
    #[cfg(feature = "wake-tflite")]
    let wake_word = State::lock(state)
        .wake_model
        .as_ref()
        .filter(|_| matches!(source, Source::Wake | Source::WakeTest))
        .map(|selection| selection.wake_word.clone());
    #[cfg(not(feature = "wake-tflite"))]
    let wake_word = {
        let _ = source;
        None
    };
    let streaming = provider.streaming();
    block_on(voice_core::run(
        &mut EspTurn {
            provider,
            audio,
            state,
            transcript: String::new(),
            streaming,
            #[cfg(feature = "wasm-plugins")]
            tools,
        },
        wav,
        #[cfg(feature = "wake-tflite")]
        wake_word.as_deref(),
        #[cfg(not(feature = "wake-tflite"))]
        wake_word,
    ))
}

#[cfg(feature = "wake-tflite")]
fn load_detector(store: &Store, storage_ready: bool) -> Result<wake::Detector> {
    if storage_ready {
        let saved = store.wake_selection().and_then(|selection| {
            selection
                .map(|selection| {
                    wake_models::Candidate::load_saved(selection)
                        .and_then(wake::Detector::prepare)
                        .and_then(wake::Detector::new)
                })
                .transpose()
        });
        match saved {
            Ok(Some(detector)) => return Ok(detector),
            Ok(None) => {}
            Err(error) => log::warn!("Restore saved wake model: {error:#}"),
        }
    }
    wake::Detector::prepare(wake_models::Candidate::builtin()).and_then(wake::Detector::new)
}

struct EspTurn<'a, 'd> {
    provider: Cascade<'a>,
    audio: &'a mut Audio<'d>,
    state: &'a Mutex<State>,
    transcript: String,
    streaming: bool,
    #[cfg(feature = "wasm-plugins")]
    tools: Option<plugins::VoiceTools<'a>>,
}

enum Speech {
    Buffered(Vec<u8>),
    Streaming(String),
}

impl Turn for EspTurn<'_, '_> {
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

    async fn synthesize(&mut self, answer: String) -> Result<Self::Speech, Failure> {
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

    async fn play(&mut self, speech: Self::Speech) -> Result<(), Failure> {
        match speech {
            Speech::Buffered(pcm) => {
                State::lock(self.state).playing(PlaybackMode::Buffered);
                self.audio.play(&pcm).map_err(playback_failure)
            }
            Speech::Streaming(text) => {
                let mut playback = self.audio.playback();
                let mut started = false;
                let mut output_failed = false;
                self.provider
                    .stream_speech(&text, |pcm| {
                        let first = !started && !pcm.is_empty();
                        if first {
                            State::lock(self.state).playing(PlaybackMode::Streaming);
                            started = true;
                        }
                        playback.push(pcm).inspect_err(|_| output_failed = true)?;
                        if first {
                            log::info!("Streamed playback started");
                        }
                        Ok(())
                    })
                    .map_err(|error| {
                        if output_failed {
                            playback_failure(error)
                        } else {
                            provider_failure(error)
                        }
                    })?;
                playback.finish().map_err(playback_failure)
            }
        }
    }

    fn no_speech(&self) -> Failure {
        Failure::NoSpeech
    }
}

fn playback_failure(error: anyhow::Error) -> Failure {
    log::warn!("Play I2S audio: {error:#}");
    Failure::Playback
}

fn provider_failure(error: anyhow::Error) -> Failure {
    log::warn!("Voice provider: {error:#}");
    Failure::ProviderFailed
}

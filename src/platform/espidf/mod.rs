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
mod worker;

use std::sync::{Arc, atomic::Ordering, mpsc};
#[cfg(feature = "wake-tflite")]
use std::time::{Duration, Instant};

use esp_idf_svc::{
    hal::{delay::FreeRtos, peripherals::Peripherals},
    nvs::EspDefaultNvsPartition,
    sys,
};
use goodwin_error::Result;

#[cfg(feature = "wake-tflite")]
use crate::panel_core::Stage;
use crate::{
    builtin_assistant::State,
    model,
    panel_core::{Failure, Source},
    voice_core::{self, CAPTURE_HZ},
};

use self::{
    audio::{Audio, Recording},
    network::Network,
    panel::{Command, Panel},
    settings::Store,
    worker::{VoiceJob, Worker},
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
        goodwin_json::to_string(&model::capabilities())?
    );
    // SAFETY: ESP-IDF initialized the external RAM before starting the Rust application.
    log::info!("PSRAM {} bytes", unsafe { sys::esp_psram_get_size() });

    let peripherals = Peripherals::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let store = Store::open(nvs.clone())?;
    let wifi = store.wifi().unwrap_or_else(|error| {
        log::warn!("Read Wi-Fi settings: {error:#}");
        None
    });
    let mut audio = model::audio(peripherals.i2s0, peripherals.pins)?;
    let network = Network::new(peripherals.modem, nvs)?;
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
    #[cfg(any(feature = "wasm-plugins", feature = "wake-tflite"))]
    let prewarm_clock = storage_ready && !cfg!(feature = "esp-emulator");
    #[cfg(not(any(feature = "wasm-plugins", feature = "wake-tflite")))]
    let prewarm_clock = false;
    network.start(wifi, panel.clone(), prewarm_clock)?;
    let healthy = !cfg!(feature = "wake-tflite") || wake_loaded;
    #[cfg(feature = "wasm-plugins")]
    let healthy = healthy && (panel.plugins.get().is_some() || !storage::has_partition());
    let worker = Worker::start(panel.clone())?;
    firmware::verify_boot(healthy)?;
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
        worker.play_ready(&mut audio, &panel);
        #[cfg(feature = "wasm-plugins")]
        if let Some(plugins) = panel.plugins.get() {
            let signals = plugins.take_signals();
            let alarm = plugins.take_timer_alarm();
            if signals != 0 || alarm.is_some() {
                #[cfg(feature = "wake-tflite")]
                if let Some(detector) = &mut detector {
                    detector.reset();
                }
                let _ = audio.set_receiving(false);
                if let Some(alarm) = alarm {
                    let played = audio.play_timer_signal();
                    match &played {
                        Ok(()) => log::info!("Played plugin timer alarm"),
                        Err(error) => log::warn!("Play plugin timer signal: {error:#}"),
                    }
                    plugins.finish_timer_alarm(alarm, played.is_ok());
                }
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
                    worker.chat(message, &panel);
                }
                Command::VoiceTest(job) => {
                    start_recording(&mut audio, Recording::ManualTest, &panel, &worker, job);
                }
                #[cfg(feature = "wake-tflite")]
                Command::WakeTest(job) => {
                    wake_test = Some((Instant::now() + Duration::from_secs(10), job));
                }
                #[cfg(feature = "wake-tflite")]
                Command::UseWakeModel(candidate, reply) => {
                    let result = (|| {
                        let prepared = wake::Detector::prepare(candidate)?;
                        wake_models::save_asset(&prepared.0, &prepared.1)?;
                        let replacement = wake::Detector::new(prepared)?;
                        #[cfg(feature = "wasm-plugins")]
                        let replacement = {
                            let mut replacement = replacement;
                            replacement.observe_with(panel.plugins.get().cloned());
                            replacement
                        };
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
            }
        }
        if audio.button_pressed()
            && panel.wifi_connected()
            && let Some(job) = start_voice(&panel, Source::PushToTalk)
        {
            #[cfg(feature = "wake-tflite")]
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            start_recording(&mut audio, Recording::Button, &panel, &worker, job);
        }
        #[cfg(feature = "wake-tflite")]
        if let Some(detector) = &mut detector {
            if let Some((deadline, job)) = &wake_test {
                let failure = job
                    .request
                    .failure()
                    .or_else(|| (Instant::now() >= *deadline).then_some(Failure::NoWake));
                if failure.is_some() {
                    State::lock(&panel.state).finish_voice(&job.request, failure);
                    wake_test = None;
                }
            }
            let (enabled, profile) = {
                let state = State::lock(&panel.state);
                (
                    (state.audio.listening || wake_test.is_some())
                        && !state.audio.microphone_muted
                        && (!state.busy() || wake_test.is_some())
                        && (wake_test.is_some() || panel.voice_ready(&state))
                        && panel.wifi_connected(),
                    wake_test.as_ref().map_or(state.audio, |(_, job)| job.audio),
                )
            };
            if enabled {
                match detector.listen(
                    &mut audio,
                    profile.wake_gain.unwrap_or(1.0) as f32,
                    profile.sensitivity as f32,
                ) {
                    Ok(Some(recording)) => {
                        let job = wake_test
                            .take()
                            .map(|(_, job)| job)
                            .or_else(|| start_voice(&panel, Source::Wake));
                        if let Some(job) = job {
                            State::lock(&panel.state).stage(Stage::Recording);
                            start_recording(&mut audio, recording, &panel, &worker, job);
                        }
                        continue;
                    }
                    Ok(_) => continue,
                    Err(error) => {
                        log::warn!("Wake capture: {error:#}");
                        if let Some((_, job)) = wake_test.take() {
                            State::lock(&panel.state)
                                .finish_voice(&job.request, Some(Failure::Microphone));
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

fn start_voice(panel: &Panel, source: Source) -> Option<VoiceJob> {
    let mut state = State::lock(&panel.state);
    if state.busy()
        || panel.firmware_busy()
        || panel.wake_model_busy()
        || state.audio.microphone_muted
        || (source == Source::Wake && !state.audio.listening)
    {
        return None;
    }
    VoiceJob::admit(&mut state, panel, source).ok()
}

fn start_recording(
    audio: &mut Audio<'_>,
    mode: Recording,
    panel: &Panel,
    worker: &Worker,
    job: VoiceJob,
) {
    let request = job.request.clone();
    let result = (|| -> std::result::Result<(), Failure> {
        request.check()?;
        let gain = job.gain;
        let mut samples = audio
            .record(mode, gain.input_gain(), || request.failure().is_some())
            .map_err(|error| {
                log::warn!("Record I2S audio: {error:#}");
                Failure::Microphone
            })?;
        request.check()?;
        if samples.len() < CAPTURE_HZ as usize / 5 {
            return Err(Failure::NoSpeech);
        }
        log::info!("Recorded {} samples", samples.len());
        gain.prepare(&mut samples);
        let wav = voice_core::recording_wav(samples).map_err(|error| {
            log::warn!("Encode recording: {error}");
            Failure::Microphone
        })?;
        worker.voice(job, wav)
    })();
    if let Err(error) = result {
        State::lock(&panel.state).finish_voice(&request, Some(error));
        if matches!(error, Failure::Cancelled) {
            log::info!("Voice turn cancelled");
        } else {
            log::warn!("Voice turn failed: {error:?}");
        }
    }
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

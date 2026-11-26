use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::wake::{Event, Frame, Model};

use crate::{
    app::{ActiveTurn, App, Failure, Source, Stage},
    voice_core::{PRE_ROLL, RecordingGain, VoiceActivity, scale_sample},
    wake::{FRAME_SAMPLES as WAKE_SAMPLES, SAMPLE_RATE as RATE, WakeModel},
    wake_catalog::Selection,
};

#[cfg(test)]
mod tests;

pub(crate) const VAD_SAMPLES: usize = RATE / 50;
const WAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Detector {
    model: Option<WakeModel>,
    frame: Vec<i16>,
    wake_frame: Vec<i16>,
    wake_gain: Option<f32>,
    history: std::collections::VecDeque<i16>,
    detection_enabled: bool,
    wake_test: Option<u64>,
    recording: Option<Recording>,
    wake_metadata: Option<Model>,
    wake_stream: u64,
    wake_index: u64,
}

struct Recording {
    id: u64,
    woke: bool,
    samples: Vec<i16>,
    activity: VoiceActivity,
    gain: RecordingGain,
}

impl Detector {
    pub(crate) fn new(model: Option<WakeModel>) -> Self {
        Self {
            model,
            frame: Vec::with_capacity(VAD_SAMPLES),
            wake_frame: Vec::with_capacity(WAKE_SAMPLES),
            wake_gain: None,
            history: std::collections::VecDeque::with_capacity(PRE_ROLL),
            detection_enabled: false,
            wake_test: None,
            recording: None,
            wake_metadata: None,
            wake_stream: 1,
            wake_index: 0,
        }
    }

    pub(crate) fn with_selection(mut self, selection: Option<&Selection>) -> Self {
        self.wake_metadata = self
            .model
            .as_ref()
            .map(|model| wake_metadata(model, selection));
        self
    }

    fn reset_wake(&mut self, app: &App) {
        self.history.clear();
        self.reset_wake_model(app);
    }

    fn reset_wake_model(&mut self, app: &App) {
        self.end_wake_stream(app);
        self.wake_frame.clear();
        if let Some(model) = self.model.as_mut() {
            let _ = model.reset();
        }
    }

    pub(crate) fn clear(&mut self, app: &App) {
        self.reset_wake(app);
        self.detection_enabled = false;
        self.wake_test = None;
        self.recording = None;
        self.frame.clear();
    }

    pub(crate) fn apply_pending_model(&mut self, app: &App) {
        if let Some(recording) = &self.recording
            && app.active().is_none_or(|turn| turn.id != recording.id)
        {
            self.recording = None;
            self.frame.clear();
            self.reset_wake(app);
        }
        if let Some(pending) = app.take_pending_wake_model() {
            log::info!("Wake model activated id={}", pending.selection.id);
            self.end_wake_stream(app);
            self.wake_metadata = Some(wake_metadata(&pending.model, Some(&pending.selection)));
            self.model = Some(pending.model);
            self.wake_frame.clear();
            self.history.clear();
            self.detection_enabled = false;
            self.wake_test = None;
            let _ = pending.activated.send(());
        }
    }

    pub(crate) fn push_sample(&mut self, sample: i16, app: &Arc<App>) {
        self.frame.push(sample);
        if self.frame.len() == VAD_SAMPLES {
            self.tick(app);
            self.frame.clear();
        }
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    fn tick(&mut self, app: &Arc<App>) {
        let active = app.active();
        if self
            .recording
            .as_ref()
            .is_some_and(|recording| active.as_ref().is_none_or(|turn| turn.id != recording.id))
        {
            self.recording = None;
            self.reset_wake(app);
        }
        if let Some(ActiveTurn {
            id,
            source,
            stage: Stage::Recording,
            transcription_gain,
            ..
        }) = active.as_ref()
        {
            let id = *id;
            if self
                .recording
                .as_ref()
                .is_none_or(|recording| recording.id != id)
            {
                self.end_wake_stream(app);
                self.recording = Some(Recording::new(
                    id,
                    *source != Source::ManualTest,
                    Vec::new(),
                    RecordingGain::new(*transcription_gain, app.default_microphone_gain),
                ));
            }
            let recording = self.recording.as_mut().expect("current recording");
            if let Some(has_speech) = recording.push(&self.frame) {
                let recording = self.recording.take().expect("completed recording");
                self.reset_wake(app);
                if has_speech {
                    app.stage(id, Stage::Transcribing);

                    let app = Arc::clone(app);
                    tokio::spawn(async move {
                        app.voice(id, recording.samples, recording.woke, recording.gain)
                            .await;
                    });
                } else {
                    app.finish(id, Some(Failure::NoSpeech));
                }
            }
            return;
        }

        if self.model.is_none() {
            return;
        }
        let wake_test = active.as_ref().and_then(|turn| {
            if turn.stage != Stage::AwaitingWake {
                return None;
            }
            if turn.started.elapsed() >= WAKE_TIMEOUT {
                app.finish(turn.id, Some(Failure::NoWake));
                return None;
            }
            Some(turn)
        });
        let (listening, sensitivity, gain) = app.configuration().wake_settings();
        let (sensitivity, gain) = wake_test.map_or((sensitivity, gain), |turn| {
            (turn.sensitivity, turn.wake_gain)
        });
        let gain = gain.unwrap_or(app.default_microphone_gain);
        let enabled = wake_test.is_some() || (active.is_none() && listening);
        let wake_test_id = wake_test.map(|turn| turn.id);
        let mode_changed = enabled != self.detection_enabled || wake_test_id != self.wake_test;
        if mode_changed {
            self.history.clear();
            self.detection_enabled = enabled;
            self.wake_test = wake_test_id;
        }
        if mode_changed || self.wake_gain != Some(gain) {
            self.reset_wake_model(app);
            self.wake_gain = Some(gain);
        }
        if !enabled {
            return;
        }
        self.buffer_wake_audio(gain);
        if self.wake_frame.len() < WAKE_SAMPLES {
            return;
        }
        let model = self.model.as_mut().expect("loaded wake model");
        let threshold = model.threshold(sensitivity);
        let prediction = model.predict(
            self.wake_frame
                .as_slice()
                .try_into()
                .expect("one wake frame"),
        );
        self.wake_frame.clear();
        let score = match prediction {
            Ok(score) => score,
            Err(error) => {
                self.end_wake_stream(app);
                log::error!("Wake inference failed: {error}");
                self.model = None;
                app.set_model(false, None);
                if let Some(id) = wake_test_id {
                    app.finish(id, Some(Failure::Model));
                }
                return;
            }
        };
        self.observe_wake_frame(app, gain, threshold, score);
        if score < threshold {
            return;
        }

        let turn = if let Some(id) = wake_test_id {
            app.stage(id, Stage::Recording);
            Ok(id)
        } else {
            app.start(Source::Wake)
        };
        if let Ok(id) = turn
            && let Some(turn) = app.active().filter(|turn| turn.id == id)
        {
            self.recording = Some(Recording::new(
                id,
                true,
                self.history.drain(..).collect(),
                RecordingGain::new(turn.transcription_gain, app.default_microphone_gain),
            ));
        }
        self.reset_wake(app);
    }

    fn observe_wake_frame(&mut self, app: &App, gain: f32, threshold: f32, score: f32) {
        let index = self.wake_index;
        self.wake_index += 1;
        let Some(model) = &self.wake_metadata else {
            return;
        };
        app.plugins.observe_wake(|| {
            let pcm: Vec<_> = self
                .history
                .iter()
                .skip(self.history.len() - WAKE_SAMPLES)
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            let ended_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            Event::Frame(Frame {
                stream: self.wake_stream,
                index,
                model: model.clone(),
                ended_at_ms,
                sample_rate: RATE as u32,
                gain,
                threshold,
                score,
                pcm: STANDARD.encode(pcm),
            })
        });
    }

    fn end_wake_stream(&mut self, app: &App) {
        if self.wake_index != 0 {
            app.plugins.observe_wake(|| Event::End {
                stream: self.wake_stream,
            });
            self.wake_stream += 1;
            self.wake_index = 0;
        }
    }

    fn buffer_wake_audio(&mut self, gain: f32) {
        let overflow = (self.history.len() + self.frame.len()).saturating_sub(PRE_ROLL);
        self.history.drain(..overflow);
        self.history.extend(&self.frame);
        self.wake_frame
            .extend(self.frame.iter().map(|&sample| scale_sample(sample, gain)));
    }
}

fn wake_metadata(model: &WakeModel, selection: Option<&Selection>) -> Model {
    Model {
        id: selection.map_or("local", |model| &model.id).to_owned(),
        wake_word: selection
            .map_or("Гудвин", |model| &model.wake_word)
            .to_owned(),
        kind: model.kind().to_owned(),
        sha256: model.sha256().to_owned(),
    }
}

impl Recording {
    fn new(id: u64, woke: bool, samples: Vec<i16>, gain: RecordingGain) -> Self {
        Self {
            id,
            woke,
            samples,
            activity: VoiceActivity::default(),
            gain,
        }
    }

    fn push(&mut self, frame: &[i16]) -> Option<bool> {
        self.samples.extend_from_slice(frame);
        self.activity.push(frame, self.gain.input_gain(), self.woke)
    }
}

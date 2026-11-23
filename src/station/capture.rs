use std::{sync::Arc, time::Duration};

use crate::{
    app::{ActiveTurn, App, Failure, Source, Stage},
    wake::{FRAME_SAMPLES as WAKE_SAMPLES, SAMPLE_RATE as RATE, WakeModel},
};

use super::gain::{RecordingGain, scale_sample};

#[cfg(test)]
mod tests;

const PRE_ROLL: usize = RATE * 6 / 5;
pub(super) const VAD_SAMPLES: usize = RATE / 50;
const FIRST_SPEECH_TIMEOUT: usize = RATE * 7 / 2;
const END_SILENCE: usize = RATE * 7 / 10;
const RECORDING_LIMIT: usize = RATE * 16;
const WAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SPEECH_RMS: i64 = 100;

pub(super) struct Detector {
    model: Option<WakeModel>,
    frame: Vec<i16>,
    wake_frame: Vec<i16>,
    wake_gain: Option<f32>,
    history: std::collections::VecDeque<i16>,
    detection_enabled: bool,
    wake_test: Option<u64>,
    recording: Option<Recording>,
}

struct Recording {
    id: u64,
    woke: bool,
    samples: Vec<i16>,
    elapsed: usize,
    voiced: usize,
    silence: usize,
    gain: RecordingGain,
}

impl Detector {
    pub(super) fn new(model: Option<WakeModel>) -> Self {
        Self {
            model,
            frame: Vec::with_capacity(VAD_SAMPLES),
            wake_frame: Vec::with_capacity(WAKE_SAMPLES),
            wake_gain: None,
            history: std::collections::VecDeque::with_capacity(PRE_ROLL),
            detection_enabled: false,
            wake_test: None,
            recording: None,
        }
    }

    fn reset_wake(&mut self) {
        self.history.clear();
        self.reset_wake_model();
    }

    fn reset_wake_model(&mut self) {
        self.wake_frame.clear();
        if let Some(model) = self.model.as_mut() {
            let _ = model.reset();
        }
    }

    pub(super) fn clear(&mut self) {
        self.reset_wake();
        self.detection_enabled = false;
        self.wake_test = None;
        self.recording = None;
        self.frame.clear();
    }

    pub(super) fn push_sample(&mut self, sample: i16, app: &Arc<App>) {
        self.frame.push(sample);
        if self.frame.len() == VAD_SAMPLES {
            self.tick(app);
            self.frame.clear();
        }
    }

    fn tick(&mut self, app: &Arc<App>) {
        let active = app.active();
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
                self.reset_wake();
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
            self.reset_wake_model();
            self.wake_gain = Some(gain);
        }
        if !enabled {
            return;
        }
        self.buffer_wake_audio(gain);
        if self.wake_frame.len() < WAKE_SAMPLES {
            return;
        }
        let prediction = self.model.as_mut().expect("loaded wake model").predict(
            self.wake_frame
                .as_slice()
                .try_into()
                .expect("one wake frame"),
        );
        self.wake_frame.clear();
        let score = match prediction {
            Ok(score) => score,
            Err(error) => {
                eprintln!("Wake inference failed: {error}");
                self.model = None;
                app.set_model(false);
                if let Some(id) = wake_test_id {
                    app.finish(id, Some(Failure::Model));
                }
                return;
            }
        };
        let threshold = 0.85 - 0.55 * sensitivity;
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
        self.reset_wake();
    }

    fn buffer_wake_audio(&mut self, gain: f32) {
        let overflow = (self.history.len() + self.frame.len()).saturating_sub(PRE_ROLL);
        self.history.drain(..overflow);
        self.history.extend(&self.frame);
        self.wake_frame
            .extend(self.frame.iter().map(|&sample| scale_sample(sample, gain)));
    }
}

impl Recording {
    fn new(id: u64, woke: bool, samples: Vec<i16>, gain: RecordingGain) -> Self {
        Self {
            id,
            woke,
            samples,
            elapsed: 0,
            voiced: 0,
            silence: 0,
            gain,
        }
    }

    fn push(&mut self, frame: &[i16]) -> Option<bool> {
        self.samples.extend_from_slice(frame);
        self.elapsed += frame.len();
        let gain = self.gain.input_gain();
        let energy: i64 = frame
            .iter()
            .map(|&sample| i64::from(scale_sample(sample, gain)).pow(2))
            .sum();
        if energy >= SPEECH_RMS * SPEECH_RMS * frame.len() as i64 {
            self.voiced += frame.len();
            self.silence = 0;
        } else {
            self.silence += frame.len();
        }
        let heard_speech = self.voiced >= RATE / 25;
        if heard_speech && (self.silence >= END_SILENCE || self.elapsed >= RECORDING_LIMIT) {
            return Some(true);
        }
        if !heard_speech && self.elapsed >= FIRST_SPEECH_TIMEOUT {
            // A short question can already be in pre-roll when the detector fires.
            return Some(self.woke);
        }
        None
    }
}

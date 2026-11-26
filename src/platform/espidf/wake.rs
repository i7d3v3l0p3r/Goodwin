use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
    time::Instant,
};

use goodwin_error::{Context, Result, bail};

use crate::{
    voice_core::{CAPTURE_HZ, PRE_ROLL, retained_wake_pre_roll, scale_sample},
    wake_catalog::{Runtime, Selection},
    wake_tflite::{FRAME_SAMPLES, TfliteModel},
};

use super::{
    audio::{Audio, FRAME_PAIRS, Recording},
    wake_models::Candidate,
};

const HISTORY_LIMIT: usize = PRE_ROLL + CAPTURE_HZ as usize * 3;

struct Frame {
    generation: u64,
    end: u64,
    samples: [i16; FRAME_SAMPLES],
}

struct Prediction {
    generation: u64,
    end: u64,
    score: Result<f32>,
    elapsed_us: u128,
}

pub(super) struct Detector {
    pub(super) selection: Selection,
    frames: SyncSender<Frame>,
    predictions: Receiver<Prediction>,
    frame_samples: usize,
    history: VecDeque<i16>,
    first: u64,
    next: u64,
    generation: u64,
    busy: bool,
    gain: Option<f32>,
    #[cfg(feature = "wasm-plugins")]
    plugins: Option<std::sync::Arc<super::plugins::Plugins>>,
}

impl Detector {
    pub(super) fn prepare(candidate: Candidate) -> Result<(Selection, TfliteModel)> {
        candidate.load()
    }

    pub(super) fn new((selection, mut model): (Selection, TfliteModel)) -> Result<Self> {
        let frame_samples = model.frame_samples();
        let (frames, input) = mpsc::sync_channel::<Frame>(1);
        let (output, predictions) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("goodwin-wake".into())
            .stack_size(16 * 1024)
            .spawn(move || {
                let mut generation = 0;
                while let Ok(frame) = input.recv() {
                    if frame.generation != generation {
                        model.reset();
                        generation = frame.generation;
                    }
                    let start = Instant::now();
                    let score = model.predict(&frame.samples[..frame_samples]);
                    let elapsed = start.elapsed();
                    crate::diagnostics::wake_frame(elapsed, score.is_err());
                    let elapsed_us = elapsed.as_micros();
                    if output
                        .send(Prediction {
                            generation,
                            end: frame.end,
                            score,
                            elapsed_us,
                        })
                        .is_err()
                    {
                        break;
                    }
                    // The capture task can already have the next frame queued.
                    // Block for a tick so this core can service its idle watchdog.
                    esp_idf_svc::hal::delay::FreeRtos::delay_ms(1);
                }
            })
            .context("Start wake inference task")?;
        Ok(Self {
            selection,
            frames,
            predictions,
            frame_samples,
            history: VecDeque::with_capacity(HISTORY_LIMIT),
            first: 0,
            next: 0,
            generation: 0,
            busy: false,
            gain: None,
            #[cfg(feature = "wasm-plugins")]
            plugins: None,
        })
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn observe_with(
        &mut self,
        plugins: Option<std::sync::Arc<super::plugins::Plugins>>,
    ) {
        self.plugins = plugins;
    }

    pub(super) fn reset(&mut self) {
        if self.gain.take().is_some() {
            self.end_observations();
            self.generation += 1;
            self.first = 0;
            self.next = 0;
            self.history.clear();
        }
    }

    pub(super) fn listen(
        &mut self,
        audio: &mut Audio<'_>,
        gain: f32,
        sensitivity: f32,
    ) -> Result<Option<Recording>> {
        if self.gain != Some(gain) {
            self.reset();
            self.gain = Some(gain);
        }
        audio.set_receiving(true)?;
        let mut mono = [0; FRAME_PAIRS];
        let samples = audio.read_mono(&mut mono)?;
        crate::diagnostics::wake_capture(samples.len());
        let overflow = (self.history.len() + samples.len()).saturating_sub(HISTORY_LIMIT);
        self.history.drain(..overflow);
        self.first += overflow as u64;
        self.history.extend(samples);
        let live = self.first + self.history.len() as u64;

        let mut discarded = false;
        match self.predictions.try_recv() {
            Ok(prediction) => {
                self.busy = false;
                if prediction.generation == self.generation {
                    let lag = live - prediction.end;
                    crate::diagnostics::wake_lag(lag);
                    if let Some(start) = retained_wake_pre_roll(self.first, prediction.end) {
                        let score = prediction.score?;
                        let baseline = self
                            .selection
                            .score_threshold
                            .unwrap_or(crate::model::WAKE_THRESHOLD);
                        let threshold = if self.selection.runtime == Runtime::MicroWakeWord {
                            baseline
                        } else {
                            (baseline + 0.55 * (0.5 - sensitivity)).clamp(0.0, 1.0)
                        };
                        #[cfg(feature = "wasm-plugins")]
                        self.observe_frame(prediction.end, lag, gain, threshold, score);
                        #[cfg(feature = "esp-emulator")]
                        log::debug!(
                            "Wake score={score:.4} inference_us={}",
                            prediction.elapsed_us
                        );
                        if score >= threshold {
                            log::info!(
                                "Wake detected score={score:.4} inference_us={}",
                                prediction.elapsed_us
                            );
                            let captured = self
                                .history
                                .iter()
                                .skip((start - self.first) as usize)
                                .copied()
                                .collect();
                            self.reset();
                            return Ok(Some(Recording::Triggered {
                                captured,
                                post_trigger: lag as usize,
                            }));
                        }
                    } else {
                        discarded = true;
                    }
                }
            }
            Err(TryRecvError::Disconnected) => bail!("Wake inference task stopped"),
            Err(TryRecvError::Empty) => {}
        }

        if !self.busy {
            let frame_samples = self.frame_samples;
            if discarded
                || retained_wake_pre_roll(self.first, self.next + frame_samples as u64).is_none()
            {
                self.end_observations();
                self.generation += 1;
                // Resume near the live edge before the detector loses the start of a request.
                let resume = live - frame_samples as u64;
                let dropped = (resume - self.next).div_ceil(frame_samples as u64);
                crate::diagnostics::capture_skipped(dropped + u64::from(discarded));
                self.next = resume;
            }
            let offset = (self.next - self.first) as usize;
            if self.history.len() >= offset + frame_samples {
                let mut samples = [0; FRAME_SAMPLES];
                for (out, &raw) in samples
                    .iter_mut()
                    .zip(self.history.range(offset..offset + frame_samples))
                {
                    *out = scale_sample(raw, gain);
                }
                let end = self.next + frame_samples as u64;
                self.frames
                    .try_send(Frame {
                        generation: self.generation,
                        end,
                        samples,
                    })
                    .map_err(|_| goodwin_error::report!("Wake inference task is unavailable"))?;
                self.next = end;
                self.busy = true;
            }
        }
        Ok(None)
    }

    fn end_observations(&self) {
        #[cfg(feature = "wasm-plugins")]
        if let Some(plugins) = &self.plugins {
            plugins.observe_wake(|| goodwin_plugin::wake::Event::End {
                stream: self.generation,
            });
        }
    }

    #[cfg(feature = "wasm-plugins")]
    fn observe_frame(&self, end: u64, lag: u64, gain: f32, threshold: f32, score: f32) {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use goodwin_plugin::wake::{Event, Frame, Model};
        use std::time::{SystemTime, UNIX_EPOCH};

        let Some(plugins) = &self.plugins else {
            return;
        };
        let start = end.saturating_sub(self.frame_samples as u64);
        if start < self.first {
            return;
        }
        plugins.observe_wake(|| {
            let start_offset = (start - self.first) as usize;
            let pcm: Vec<u8> = self
                .history
                .range(start_offset..start_offset + self.frame_samples)
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            let captured_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let lag_ms = lag * 1_000 / u64::from(CAPTURE_HZ);
            Event::Frame(Frame {
                stream: self.generation,
                index: start / self.frame_samples as u64,
                model: Model {
                    id: self.selection.id.clone(),
                    wake_word: self.selection.wake_word.clone(),
                    kind: crate::model::WAKE_RUNTIME.into(),
                    sha256: self.selection.sha256.clone(),
                },
                ended_at_ms: captured_ms.saturating_sub(lag_ms),
                sample_rate: CAPTURE_HZ,
                gain,
                threshold,
                score,
                pcm: STANDARD.encode(pcm),
            })
        });
    }
}

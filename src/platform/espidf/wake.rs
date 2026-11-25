use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
    time::Instant,
};

use anyhow::{Context, Result, bail};

use crate::{
    voice_core::{CAPTURE_HZ, PRE_ROLL, scale_sample},
    wake_catalog::Selection,
    wake_tflite::{FRAME_SAMPLES, TfliteModel},
};

use super::{
    audio::{Audio, FRAME_PAIRS, Recording},
    wake_models::Candidate,
};

const MODEL_SIZE: usize = include_bytes!(concat!(env!("OUT_DIR"), "/wake.tflite")).len();
const HISTORY_LIMIT: usize = PRE_ROLL + CAPTURE_HZ as usize * 3;

#[repr(align(16))]
struct AlignedModel([u8; MODEL_SIZE]);

static MODEL: AlignedModel =
    AlignedModel(*include_bytes!(concat!(env!("OUT_DIR"), "/wake.tflite")));

struct Frame {
    generation: u64,
    end: usize,
    samples: [i16; FRAME_SAMPLES],
}

struct Prediction {
    generation: u64,
    end: usize,
    score: Result<f32>,
    elapsed_us: u128,
}

pub(super) struct Detector {
    pub(super) selection: Selection,
    frames: SyncSender<Frame>,
    predictions: Receiver<Prediction>,
    history: VecDeque<i16>,
    first: usize,
    next: usize,
    generation: u64,
    busy: bool,
    gain: Option<f32>,
    #[cfg(feature = "wasm-plugins")]
    plugins: Option<std::sync::Arc<super::plugins::Plugins>>,
}

impl Detector {
    pub(super) fn prepare(candidate: Candidate) -> Result<(Selection, TfliteModel)> {
        candidate.load(MODEL.0.as_slice())
    }

    pub(super) fn new((selection, mut model): (Selection, TfliteModel)) -> Result<Self> {
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
                    let score = model.predict(&frame.samples);
                    let elapsed_us = start.elapsed().as_micros();
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
        let overflow = (self.history.len() + samples.len()).saturating_sub(HISTORY_LIMIT);
        self.history.drain(..overflow);
        self.first += overflow;
        self.history.extend(samples);

        match self.predictions.try_recv() {
            Ok(prediction) => {
                self.busy = false;
                if prediction.generation == self.generation && prediction.end > self.first {
                    let score = prediction.score?;
                    let baseline = self
                        .selection
                        .score_threshold
                        .unwrap_or(crate::model::WAKE_THRESHOLD);
                    let threshold = (baseline + 0.55 * (0.5 - sensitivity)).clamp(0.0, 1.0);
                    #[cfg(feature = "wasm-plugins")]
                    self.observe_frame(prediction.end, gain, threshold, score);
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
                        let start = prediction.end.saturating_sub(PRE_ROLL).max(self.first);
                        let captured = self
                            .history
                            .iter()
                            .skip(start - self.first)
                            .copied()
                            .collect();
                        let post_trigger = self.first + self.history.len() - prediction.end;
                        self.reset();
                        return Ok(Some(Recording::Wake {
                            captured,
                            post_trigger,
                        }));
                    }
                }
            }
            Err(TryRecvError::Disconnected) => bail!("Wake inference task stopped"),
            Err(TryRecvError::Empty) => {}
        }

        if !self.busy {
            if self.next < self.first {
                self.end_observations();
                self.generation += 1;
                // The worker missed audio. Resume near the live edge with fresh model state.
                self.next = self.first + self.history.len() - FRAME_SAMPLES;
            }
            let offset = self.next - self.first;
            if self.history.len() >= offset + FRAME_SAMPLES {
                let mut samples = [0; FRAME_SAMPLES];
                for (out, &raw) in samples
                    .iter_mut()
                    .zip(self.history.range(offset..offset + FRAME_SAMPLES))
                {
                    *out = scale_sample(raw, gain);
                }
                let end = self.next + FRAME_SAMPLES;
                self.frames
                    .try_send(Frame {
                        generation: self.generation,
                        end,
                        samples,
                    })
                    .map_err(|_| anyhow::anyhow!("Wake inference task is unavailable"))?;
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
    fn observe_frame(&self, end: usize, gain: f32, threshold: f32, score: f32) {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use goodwin_plugin::wake::{Event, Frame, Model};
        use std::time::{SystemTime, UNIX_EPOCH};

        let Some(plugins) = &self.plugins else {
            return;
        };
        let start = end.saturating_sub(FRAME_SAMPLES);
        if start < self.first {
            return;
        }
        plugins.observe_wake(|| {
            let start_offset = start - self.first;
            let pcm: Vec<u8> = self
                .history
                .range(start_offset..start_offset + FRAME_SAMPLES)
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            let captured_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let lag_ms =
                (self.first + self.history.len() - end) as u64 * 1_000 / u64::from(CAPTURE_HZ);
            Event::Frame(Frame {
                stream: self.generation,
                index: (start / FRAME_SAMPLES) as u64,
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

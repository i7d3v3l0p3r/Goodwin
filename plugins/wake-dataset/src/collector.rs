use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::wake::{Event, Frame, Model};

use crate::store::{ClipInfo, FRAME_SAMPLES, LIMIT, SAMPLE_RATE, Store, WINDOW_FRAMES};

const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
const STRIDE: usize = WINDOW_FRAMES / 2;

#[derive(Default)]
pub struct Collector(Option<Window>);

struct Window {
    stream: u64,
    next_index: u64,
    model: Model,
    gain: f32,
    threshold: f32,
    ended_at_ms: u64,
    pcm: Vec<u8>,
    scores: Vec<f32>,
    new_frames: usize,
}

impl Collector {
    pub fn observe(&mut self, store: &mut Store, event: Event, minimum_score: f32) -> Result<()> {
        let frame = match event {
            Event::Frame(frame) => frame,
            Event::End { stream } => {
                if self
                    .0
                    .as_ref()
                    .is_some_and(|window| window.stream == stream)
                {
                    self.flush(store, minimum_score)?;
                }
                return Ok(());
            }
        };
        let pcm = STANDARD.decode(&frame.pcm)?;
        ensure!(
            frame.sample_rate == SAMPLE_RATE && pcm.len() == FRAME_BYTES,
            "Unsupported wake audio format"
        );
        if self
            .0
            .as_ref()
            .is_some_and(|window| !window.continues(&frame))
        {
            self.flush(store, minimum_score)?;
        }
        if store.clips.len() >= LIMIT {
            return Ok(());
        }
        let window = self.0.get_or_insert_with(|| Window {
            stream: frame.stream,
            next_index: frame.index,
            model: frame.model,
            gain: frame.gain,
            threshold: frame.threshold,
            ended_at_ms: frame.ended_at_ms,
            pcm: Vec::with_capacity(WINDOW_FRAMES * FRAME_BYTES),
            scores: Vec::with_capacity(WINDOW_FRAMES),
            new_frames: 0,
        });
        window.next_index = frame.index + 1;
        window.ended_at_ms = frame.ended_at_ms;
        window.pcm.extend(pcm);
        window.scores.push(frame.score);
        window.new_frames += 1;
        if window.scores.len() == WINDOW_FRAMES {
            window.save_if_qualified(store, minimum_score)?;
            window.pcm.drain(..STRIDE * FRAME_BYTES);
            window.scores.drain(..STRIDE);
            window.new_frames = 0;
        }
        Ok(())
    }

    pub fn flush(&mut self, store: &mut Store, minimum_score: f32) -> Result<()> {
        if let Some(window) = self.0.take()
            && store.clips.len() < LIMIT
        {
            window.save_if_qualified(store, minimum_score)?;
        }
        Ok(())
    }
}

impl Window {
    fn continues(&self, frame: &Frame) -> bool {
        self.stream == frame.stream
            && self.next_index == frame.index
            && self.model == frame.model
            && self.gain == frame.gain
            && self.threshold == frame.threshold
    }

    fn save_if_qualified(&self, store: &mut Store, minimum_score: f32) -> Result<()> {
        let overlap = self.scores.len() - self.new_frames;
        if !self.scores[overlap..]
            .iter()
            .any(|&score| score >= minimum_score)
        {
            return Ok(());
        }
        let duration_ms = (self.scores.len() * FRAME_SAMPLES * 1000 / SAMPLE_RATE as usize) as u64;
        store.save(
            ClipInfo {
                model: self.model.clone(),
                captured_at_ms: self.ended_at_ms.saturating_sub(duration_ms),
                sample_rate: SAMPLE_RATE,
                frame_samples: FRAME_SAMPLES,
                gain: self.gain,
                threshold: self.threshold,
                scores: self.scores.clone(),
            },
            &self.pcm,
        )
    }
}

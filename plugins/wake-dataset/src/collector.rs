use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Result, ensure};
use goodwin_plugin::{
    audio::scale_sample,
    wake::{Event, Frame, Model},
};

use crate::store::{ClipInfo, SAMPLE_RATE, Store, window_frames};

#[derive(Default)]
pub struct Collector(Option<Window>);

struct Window {
    stream: u64,
    next_index: u64,
    model: Model,
    gain: f32,
    threshold: f32,
    ended_at_ms: u64,
    frame_samples: usize,
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
        let mut pcm = STANDARD.decode(&frame.pcm)?;
        let frame_samples = pcm.len() / 2;
        let frames = window_frames(frame_samples);
        ensure!(
            frame.sample_rate == SAMPLE_RATE && pcm.len().is_multiple_of(2) && frames.is_some(),
            "Unsupported wake audio format"
        );
        let frames = frames.expect("validated wake audio");
        if self
            .0
            .as_ref()
            .is_some_and(|window| !window.continues(&frame, frame_samples))
        {
            self.flush(store, minimum_score)?;
        }
        if store.full() {
            return Ok(());
        }
        if frame.gain != 1.0 {
            for bytes in pcm.as_chunks_mut::<2>().0 {
                let sample = i16::from_le_bytes(*bytes);
                *bytes = scale_sample(sample, frame.gain).to_le_bytes();
            }
        }
        let window = self.0.get_or_insert_with(|| Window {
            stream: frame.stream,
            next_index: frame.index,
            model: frame.model,
            gain: frame.gain,
            threshold: frame.threshold,
            ended_at_ms: frame.ended_at_ms,
            frame_samples,
            pcm: Vec::with_capacity(frames * frame_samples * 2),
            scores: Vec::with_capacity(frames),
            new_frames: 0,
        });
        window.next_index = frame.index + 1;
        window.ended_at_ms = frame.ended_at_ms;
        window.pcm.extend(pcm);
        window.scores.push(frame.score);
        window.new_frames += 1;
        if window.scores.len() == frames {
            window.save_if_qualified(store, minimum_score)?;
            let stride = frames / 2;
            window.pcm.drain(..stride * frame_samples * 2);
            window.scores.drain(..stride);
            window.new_frames = 0;
        }
        Ok(())
    }

    pub fn flush(&mut self, store: &mut Store, minimum_score: f32) -> Result<()> {
        if let Some(window) = self.0.take()
            && !store.full()
        {
            window.save_if_qualified(store, minimum_score)?;
        }
        Ok(())
    }
}

impl Window {
    fn continues(&self, frame: &Frame, frame_samples: usize) -> bool {
        self.stream == frame.stream
            && self.next_index == frame.index
            && self.model == frame.model
            && self.frame_samples == frame_samples
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
        let duration_ms =
            (self.scores.len() * self.frame_samples * 1000 / SAMPLE_RATE as usize) as u64;
        store.save(
            ClipInfo {
                model: self.model.clone(),
                captured_at_ms: self.ended_at_ms.saturating_sub(duration_ms),
                sample_rate: SAMPLE_RATE,
                frame_samples: self.frame_samples,
                gain: self.gain,
                gain_applied: true,
                threshold: self.threshold,
                scores: self.scores.clone(),
            },
            &self.pcm,
        )
    }
}

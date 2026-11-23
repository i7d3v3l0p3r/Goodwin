use std::{
    sync::{Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::app::{Source, Stage};
use crate::wav::PcmAudio;

mod hardware;

use hardware::Hardware;

const FRAME_MS: u64 = 16;
const RING_PIXELS: usize = 6;
const SEGMENTS: usize = 32;
const MAX_MOUTH_LEVEL: u8 = 31;
const MOUTH_ATTACK_MS: f32 = 35.0;
const MOUTH_RELEASE_MS: f32 = 50.0;

// Digits are ordered from left to right, with A through G within each digit.
const fn segment(digit: usize, letter: u8) -> usize {
    digit * 7 + (letter - b'A') as usize
}

struct Frame {
    ring: [u8; RING_PIXELS * 3],
    clock: [u8; SEGMENTS],
}

enum Scene {
    Idle,
    Listening,
    Thinking,
    Speaking { start: Instant, levels: Vec<u8> },
    SpeechLevels(Vec<u8>),
    Stop,
}

pub(crate) struct Indicators {
    worker: Option<Worker>,
}

struct Worker {
    events: mpsc::Sender<Scene>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Indicators {
    pub(crate) fn new() -> Self {
        if cfg!(test) {
            return Self { worker: None };
        }
        let worker = Hardware::detect().and_then(|hardware| {
            let (events, receiver) = mpsc::channel();
            let thread = thread::Builder::new()
                .name("goodwin-indicators".into())
                .spawn(move || run(hardware, receiver))
                .map_err(|error| eprintln!("Could not start the Station indicators: {error}"))
                .ok()?;
            Some(Worker {
                events,
                thread: Mutex::new(Some(thread)),
            })
        });
        Self { worker }
    }

    pub(crate) fn stage(&self, source: Source, stage: Stage) {
        if source == Source::Chat {
            return;
        }
        match stage {
            Stage::AwaitingWake | Stage::Playing => {}
            Stage::Recording => self.send(Scene::Listening),
            Stage::Transcribing | Stage::Thinking | Stage::Synthesizing => {
                self.send(Scene::Thinking);
            }
        }
    }

    pub(crate) fn finish(&self, source: Source) {
        if source != Source::Chat {
            self.send(Scene::Idle);
        }
    }

    pub(super) fn envelope(&self, wave: &[u8]) -> Option<Vec<u8>> {
        self.worker.as_ref()?;
        Some(speech_levels(wave).unwrap_or_default())
    }

    pub(super) fn speak(&self, levels: Vec<u8>) {
        self.send(Scene::Speaking {
            start: Instant::now(),
            levels,
        });
    }

    pub(super) fn begin_stream(&self, audio: &PcmAudio) -> Option<SpeechEnvelope> {
        self.worker.as_ref()?;
        let mut envelope = SpeechEnvelope::new(
            u64::from(audio.sample_rate),
            usize::from(audio.channels) * 2,
        );
        self.speak(envelope.push(&audio.data));
        Some(envelope)
    }

    pub(super) fn append_levels(&self, levels: Vec<u8>) {
        if !levels.is_empty() {
            self.send(Scene::SpeechLevels(levels));
        }
    }

    pub(crate) fn stop(&self) {
        if let Some(worker) = &self.worker
            && let Some(thread) = worker.thread.lock().expect("indicator worker").take()
        {
            let _ = worker.events.send(Scene::Stop);
            let _ = thread.join();
        }
    }

    fn send(&self, scene: Scene) {
        if let Some(worker) = &self.worker {
            let _ = worker.events.send(scene);
        }
    }
}

impl Drop for Indicators {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(hardware: Hardware, receiver: mpsc::Receiver<Scene>) {
    while let Ok(mut scene) = receiver.recv() {
        match scene {
            Scene::Stop => return,
            Scene::Idle | Scene::SpeechLevels(_) => continue,
            _ => {}
        }
        let mut lights = hardware.begin();
        let mut since = Instant::now();
        loop {
            let elapsed = match &scene {
                Scene::Speaking { start, .. } => start.elapsed(),
                _ => since.elapsed(),
            };
            lights.show(&draw(&scene, elapsed));
            match receiver.recv_timeout(Duration::from_millis(FRAME_MS)) {
                Ok(Scene::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Ok(Scene::Idle) => break,
                Ok(Scene::Thinking) if matches!(scene, Scene::Thinking) => {}
                Ok(Scene::SpeechLevels(chunk)) => {
                    if let Scene::Speaking { start, levels } = &mut scene {
                        let current = (start.elapsed().as_millis() / u128::from(FRAME_MS)) as usize;
                        levels.resize(levels.len().max(current), 0);
                        levels.extend(chunk);
                    }
                }
                Ok(next) => {
                    scene = next;
                    since = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn draw(scene: &Scene, elapsed: Duration) -> Frame {
    const RING_COLOR: [u8; 3] = [48, 38, 6]; // BGR framebuffer order.
    let mut frame = Frame {
        ring: [0; RING_PIXELS * 3],
        clock: [0; SEGMENTS],
    };
    let blink = matches!(scene, Scene::Listening)
        && (1_650..1_780).contains(&(elapsed.as_millis() % 3_200));
    let eye = if blink { b"G".as_slice() } else { b"ABFG" };
    for digit in [0, 3] {
        for &letter in eye {
            frame.clock[segment(digit, letter)] = 255;
        }
    }

    let head = elapsed.as_secs_f32() * RING_PIXELS as f32 / 1.15;
    for (index, pixel) in frame.ring.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let intensity = match scene {
            Scene::Thinking => {
                let distance = (head - index as f32).rem_euclid(RING_PIXELS as f32);
                let trail = (1.0 - distance.min(RING_PIXELS as f32 - distance) / 1.8).max(0.0);
                0.08 + 0.92 * trail * trail
            }
            Scene::Speaking { .. } => 0.24,
            _ => 0.68,
        };
        for (channel, value) in pixel.iter_mut().zip(RING_COLOR) {
            *channel = (f32::from(value) * intensity).round() as u8;
        }
    }

    if let Scene::Speaking { levels, .. } = scene {
        let index = (elapsed.as_millis() / u128::from(FRAME_MS)) as usize;
        let level = levels.get(index).copied().unwrap_or(0);
        let mouth = (u16::from(level) * 255 / u16::from(MAX_MOUTH_LEVEL)) as u8;
        for digit in 0..4 {
            frame.clock[segment(digit, b'D')] = mouth;
        }
    }
    frame
}

fn speech_levels(wave: &[u8]) -> Option<Vec<u8>> {
    if wave.get(..4)? != b"RIFF" || wave.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut chunks = wave.get(12..)?;
    let mut format = None;
    let mut pcm = None;
    while chunks.len() >= 8 {
        let (header, body) = chunks.split_at(8);
        let len = u32::from_le_bytes(header[4..].try_into().ok()?) as usize;
        let data = body.get(..len)?;
        match &header[..4] {
            b"fmt " if data.len() >= 16 => {
                let encoding = u16::from_le_bytes(data[..2].try_into().ok()?);
                let channels = u16::from_le_bytes(data[2..4].try_into().ok()?);
                let rate = u32::from_le_bytes(data[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes(data[14..16].try_into().ok()?);
                if encoding != 1
                    || bits != 16
                    || !(1..=2).contains(&channels)
                    || !(8_000..=96_000).contains(&rate)
                {
                    return None;
                }
                format = Some((u64::from(rate), usize::from(channels) * 2));
            }
            b"data" => pcm = Some(data),
            _ => {}
        }
        if format.is_some() && pcm.is_some() {
            break;
        }
        chunks = body.get(len.checked_add(len % 2)?..)?;
    }

    let (rate, block_align) = format?;
    let mut envelope = SpeechEnvelope::new(rate, block_align);
    let mut levels = envelope.push(pcm?);
    levels.extend(envelope.finish());
    Some(levels)
}

pub(super) struct SpeechEnvelope {
    rate: u64,
    channels: usize,
    frame: u64,
    samples: usize,
    square_sum: f32,
    previous: f32,
    attack: f32,
    release: f32,
}

impl SpeechEnvelope {
    fn new(rate: u64, block_align: usize) -> Self {
        Self {
            rate,
            channels: block_align / 2,
            frame: 0,
            samples: 0,
            square_sum: 0.0,
            previous: 0.0,
            attack: 1.0 - (-(FRAME_MS as f32) / MOUTH_ATTACK_MS).exp(),
            release: 1.0 - (-(FRAME_MS as f32) / MOUTH_RELEASE_MS).exp(),
        }
    }

    pub(super) fn push(&mut self, pcm: &[u8]) -> Vec<u8> {
        let mut levels = Vec::new();
        let mut pcm = pcm.as_chunks::<2>().0;
        while !pcm.is_empty() {
            let frame_samples = (((self.frame + 1) * self.rate * FRAME_MS / 1_000
                - self.frame * self.rate * FRAME_MS / 1_000)
                as usize)
                * self.channels;
            let count = (frame_samples - self.samples).min(pcm.len());
            let (samples, remaining) = pcm.split_at(count);
            self.square_sum += samples
                .iter()
                .map(|&sample| f32::from(i16::from_le_bytes(sample)).powi(2))
                .sum::<f32>();
            self.samples += count;
            pcm = remaining;
            if self.samples == frame_samples {
                levels.push(self.level());
                self.frame += 1;
            }
        }
        levels
    }

    pub(super) fn finish(&mut self) -> Option<u8> {
        (self.samples != 0).then(|| self.level())
    }

    fn level(&mut self) -> u8 {
        let rms = (self.square_sum / self.samples as f32).sqrt();
        let target = ((rms - 240.0) / 6_500.0).clamp(0.0, 1.0).sqrt();
        let response = if target > self.previous {
            self.attack
        } else {
            self.release
        };
        self.previous += (target - self.previous) * response;
        self.samples = 0;
        self.square_sum = 0.0;
        (self.previous * f32::from(MAX_MOUTH_LEVEL)).round() as u8
    }
}

#[cfg(test)]
mod tests;

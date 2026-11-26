use std::{
    future::Future,
    io,
    sync::{Arc, Mutex},
};

#[cfg(not(target_os = "espidf"))]
use std::{
    future::poll_fn,
    task::{Poll, Waker},
};

use crate::panel_core::{Failure, Source, Stage};

pub(crate) fn first_voice_stage(source: Source) -> Stage {
    if source == Source::WakeTest {
        Stage::AwaitingWake
    } else {
        Stage::Recording
    }
}

#[derive(Clone)]
pub(crate) struct VoiceRequest {
    pub(crate) id: u64,
    state: Arc<Mutex<Cancellation>>,
}

#[derive(Default)]
struct Cancellation {
    reason: Option<Failure>,
    #[cfg(not(target_os = "espidf"))]
    waiter: Option<Waker>,
}

impl VoiceRequest {
    pub(crate) fn new(id: u64) -> Self {
        Self {
            id,
            state: Arc::default(),
        }
    }

    pub(crate) fn cancel(&self, reason: Failure) {
        let mut state = self.state.lock().expect("voice cancellation");
        if state.reason.is_none() {
            state.reason = Some(reason);
            crate::diagnostics::voice_cancelled();
        }
        #[cfg(not(target_os = "espidf"))]
        if let Some(waiter) = state.waiter.take() {
            drop(state);
            waiter.wake();
        }
    }

    pub(crate) fn failure(&self) -> Option<Failure> {
        self.state.lock().expect("voice cancellation").reason
    }

    pub(crate) fn check(&self) -> Result<(), Failure> {
        self.failure().map_or(Ok(()), Err)
    }

    #[cfg(not(target_os = "espidf"))]
    async fn cancelled(&self) -> Failure {
        poll_fn(|cx| {
            let mut state = self.state.lock().expect("voice cancellation");
            if let Some(reason) = state.reason {
                Poll::Ready(reason)
            } else {
                state.waiter = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }

    pub(crate) async fn step<T>(
        &self,
        work: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        self.check()?;
        #[cfg(not(target_os = "espidf"))]
        let result = {
            let result = tokio::select! {
                biased;
                reason = self.cancelled() => Err(reason),
                result = work => result,
            };
            // The request itself is kept by the task waiting here.
            self.state.lock().expect("voice cancellation").waiter = None;
            result
        };
        #[cfg(target_os = "espidf")]
        let result = work.await;
        self.check()?;
        result
    }
}

#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
pub(crate) struct VoiceProviders<T> {
    pub(crate) transcription: T,
    pub(crate) chat: T,
    pub(crate) speech: T,
}

#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
impl<T> VoiceProviders<T> {
    pub(crate) fn resolve<E>(
        mut resolve: impl FnMut(goodwin_plugin::provider::Capability) -> Result<T, E>,
    ) -> Result<Self, (Stage, E)> {
        use goodwin_plugin::provider::Capability;
        Ok(Self {
            transcription: resolve(Capability::Transcription)
                .map_err(|error| (Stage::Transcribing, error))?,
            chat: resolve(Capability::Chat).map_err(|error| (Stage::Thinking, error))?,
            speech: resolve(Capability::Speech).map_err(|error| (Stage::Synthesizing, error))?,
        })
    }
}

pub(crate) use goodwin_plugin::audio::scale_sample;
use goodwin_plugin::audio::wav_header;

mod gain;
#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
pub(crate) mod tools;
pub(crate) use gain::RecordingGain;

pub(crate) const CAPTURE_HZ: u32 = 16_000;
pub(crate) const RECORDING_LIMIT: usize = CAPTURE_HZ as usize * 16;

#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
pub(crate) fn timer_signal_pcm(sample_rate: u32) -> Vec<u8> {
    let count = sample_rate / 3;
    let fade = sample_rate as f32 / 50.0;
    (0..count)
        .flat_map(|index| {
            let time = index as f32 / sample_rate as f32;
            let envelope = (index.min(count - index) as f32 / fade).min(1.0);
            let angle = std::f32::consts::TAU * time;
            let sample =
                envelope * ((angle * 660.0).sin() * 5000.0 + (angle * 880.0).sin() * 2500.0);
            (sample as i16).to_le_bytes()
        })
        .collect()
}
#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
pub(crate) const PRE_ROLL: usize = CAPTURE_HZ as usize * 6 / 5;
pub(crate) const FIRST_SPEECH_TIMEOUT: usize = CAPTURE_HZ as usize * 7 / 2;
pub(crate) const END_SILENCE: usize = CAPTURE_HZ as usize * 7 / 10;
pub(crate) const VOICE_INSTRUCTIONS: &str = "You are Goodwin, a voice assistant. Reply in Russian in concise plain text suitable for speaking aloud.";

pub(crate) fn instructions(prompt: &str) -> std::borrow::Cow<'_, str> {
    if prompt.is_empty() {
        VOICE_INSTRUCTIONS.into()
    } else {
        format!("{prompt}\n\nUse plain text suitable for speaking aloud.").into()
    }
}

#[derive(Default)]
pub(crate) struct VoiceActivity {
    elapsed: usize,
    voiced: usize,
    silence: usize,
}

impl VoiceActivity {
    pub(crate) fn push(&mut self, frame: &[i16], gain: f32, has_pre_roll: bool) -> Option<bool> {
        const RATE: usize = CAPTURE_HZ as usize;
        const SPEECH_RMS: i64 = 100;
        self.elapsed += frame.len();
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
            return Some(has_pre_roll);
        }
        None
    }
}

pub(crate) trait Turn {
    type Speech: Send;

    fn stage(&mut self, stage: Stage);
    fn transcribe(&mut self, wav: &[u8]) -> impl Future<Output = Result<String, Failure>> + Send;
    fn reply(&mut self, text: &str) -> impl Future<Output = Result<String, Failure>> + Send;
    fn synthesize(
        &mut self,
        answer: String,
    ) -> impl Future<Output = Result<Self::Speech, Failure>> + Send;
    fn play(&mut self, speech: Self::Speech) -> impl Future<Output = Result<(), Failure>> + Send;
}

pub(crate) async fn run<T: Turn>(
    request: &VoiceRequest,
    turn: &mut T,
    wav: RecordingWav,
    wake_word: Option<&str>,
) -> Result<(), Failure> {
    request.check()?;
    turn.stage(Stage::Transcribing);
    let text = request.step(turn.transcribe(wav.as_bytes())).await?;
    drop(wav);
    let text = wake_word.map_or_else(|| text.trim(), |word| strip_wake_word(&text, word));
    if text.is_empty() {
        return Err(Failure::NoSpeech);
    }
    turn.stage(Stage::Thinking);
    let answer = request.step(turn.reply(text)).await?;
    turn.stage(Stage::Synthesizing);
    let speech = request.step(turn.synthesize(answer)).await?;
    request.step(turn.play(speech)).await
}

pub(crate) struct RecordingWav(Vec<i16>);

impl RecordingWav {
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: The initialized i16 buffer remains owned by self. Every
        // initialized byte is valid as a u8 and requires no extra alignment.
        unsafe {
            std::slice::from_raw_parts(self.0.as_ptr().cast(), self.0.len() * size_of::<i16>())
        }
    }
}

pub(crate) fn recording_wav(mut samples: Vec<i16>) -> io::Result<RecordingWav> {
    let sample_count = samples.len();
    let header = wav_header(CAPTURE_HZ, 1, sample_count * size_of::<i16>())?;
    let header_samples = header.len() / size_of::<i16>();
    samples
        .try_reserve_exact(header_samples)
        .map_err(io::Error::other)?;
    samples.resize(sample_count + header_samples, 0);
    samples.copy_within(..sample_count, header_samples);
    let (words, _) = header.as_chunks::<2>();
    for (sample, &bytes) in samples.iter_mut().zip(words) {
        *sample = i16::from_ne_bytes(bytes);
    }
    #[cfg(target_endian = "big")]
    for sample in &mut samples[header_samples..] {
        *sample = sample.to_le();
    }
    // Capture grows the buffer in chunks. Release spare capacity before STT.
    samples.shrink_to_fit();
    Ok(RecordingWav(samples))
}

pub(crate) fn strip_wake_word<'a>(text: &'a str, wake_word: &str) -> &'a str {
    let delimiter = |c: char| c.is_whitespace() || ",.!?;:—–-«»\"".contains(c);
    let text = text.trim().trim_start_matches(delimiter);
    let mut remaining = text;
    for word in wake_word.split(delimiter).filter(|word| !word.is_empty()) {
        remaining = remaining.trim_start_matches(delimiter);
        let first = remaining.split(delimiter).next().unwrap_or_default();
        let expected = word.to_lowercase();
        let actual = first.to_lowercase();
        if actual != expected && !(expected == "гудвин" && actual == "goodwin") {
            return text;
        }
        remaining = &remaining[first.len()..];
    }
    remaining.trim_start_matches(delimiter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "espidf"))]
    #[tokio::test]
    async fn cancelling_a_pending_step_drops_its_work_and_keeps_the_first_reason() {
        let request = VoiceRequest::new(1);
        let (entered, running) = tokio::sync::oneshot::channel();
        let (alive, mut dropped) = tokio::sync::oneshot::channel::<()>();
        let active = request.clone();
        let worker = tokio::spawn(async move {
            active
                .step(async move {
                    let _ = entered.send(());
                    let result = std::future::pending::<Result<(), Failure>>().await;
                    drop(alive);
                    result
                })
                .await
        });
        running.await.unwrap();
        assert!(
            dropped
                .try_recv()
                .is_err_and(|error| error == tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        request.cancel(Failure::Cancelled);
        request.cancel(Failure::MicrophoneMuted);
        assert!(matches!(worker.await.unwrap(), Err(Failure::Cancelled)));
        assert!(dropped.await.is_err());
        assert!(matches!(
            request
                .step(async { panic!("a cancelled request must not start another step") })
                .await,
            Err::<(), _>(Failure::Cancelled)
        ));
    }

    #[test]
    fn recording_wav_reuses_the_capture_buffer_and_preserves_pcm() {
        let mut samples = Vec::with_capacity(4 + 44 / size_of::<i16>());
        samples.extend([i16::MIN, -1, 0, i16::MAX]);
        let capture = samples.as_ptr().cast::<u8>();
        let recording = recording_wav(samples).unwrap();
        let wav = recording.as_bytes();
        assert_eq!(wav.as_ptr(), capture);
        assert_eq!(&wav[..12], b"RIFF\x2c\0\0\0WAVE");
        assert_eq!(&wav[12..24], b"fmt \x10\0\0\0\x01\0\x01\0");
        assert_eq!(&wav[24..28], &16_000_u32.to_le_bytes());
        assert_eq!(&wav[28..36], b"\x00\x7d\0\0\x02\0\x10\0");
        assert_eq!(&wav[36..44], b"data\x08\0\0\0");
        assert_eq!(&wav[44..], &[0, 0x80, 0xff, 0xff, 0, 0, 0xff, 0x7f]);
    }
}

use std::{future::Future, io};

use goodwin_plugin::audio::PcmAudio;

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

pub(crate) fn scale_sample(sample: i16, gain: f32) -> i16 {
    // Float-to-integer casts saturate at the PCM limits.
    (f32::from(sample) * gain) as i16
}

pub(crate) trait Turn {
    type Error;
    type Speech: Send;

    fn transcribe(
        &mut self,
        wav: &[u8],
    ) -> impl Future<Output = Result<String, Self::Error>> + Send;
    fn reply(&mut self, text: &str) -> impl Future<Output = Result<String, Self::Error>> + Send;
    fn synthesize(
        &mut self,
        answer: String,
    ) -> impl Future<Output = Result<Self::Speech, Self::Error>> + Send;
    fn play(
        &mut self,
        speech: Self::Speech,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn no_speech(&self) -> Self::Error;
}

pub(crate) async fn run<T: Turn>(
    turn: &mut T,
    wav: Vec<u8>,
    wake_word: Option<&str>,
) -> Result<(), T::Error> {
    let text = turn.transcribe(&wav).await?;
    drop(wav);
    let text = wake_word.map_or_else(|| text.trim(), |word| strip_wake_word(&text, word));
    if text.is_empty() {
        return Err(turn.no_speech());
    }
    let answer = turn.reply(text).await?;
    let speech = turn.synthesize(answer).await?;
    turn.play(speech).await
}

pub(crate) fn recording_wav(samples: Vec<i16>) -> io::Result<Vec<u8>> {
    let recording = PcmAudio {
        data: samples.into_iter().flat_map(i16::to_le_bytes).collect(),
        sample_rate: CAPTURE_HZ,
        channels: 1,
    };
    let mut wav = Vec::with_capacity(recording.data.len() + 44);
    recording.write_wav(&mut wav)?;
    Ok(wav)
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

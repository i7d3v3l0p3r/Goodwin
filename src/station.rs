mod capture;
mod gain;
mod indicators;
mod microphone;
mod playback;
mod protocol;

#[cfg(test)]
mod tests;

pub(crate) use gain::RecordingGain;
pub(crate) use indicators::Indicators;
pub use microphone::run_microphone;
pub use playback::Player;
pub(crate) use playback::timer_signal;

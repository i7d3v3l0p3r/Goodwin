mod capture;
mod microphone;
mod playback;
mod protocol;

#[cfg(test)]
mod tests;

pub use microphone::{MicrophoneOptions, run_microphone};
pub use playback::Player;
pub(crate) use playback::timer_signal;

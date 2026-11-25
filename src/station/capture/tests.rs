use super::{Detector, PRE_ROLL, Recording, RecordingGain, VAD_SAMPLES};
use crate::voice_core::{END_SILENCE, FIRST_SPEECH_TIMEOUT};

#[test]
fn pre_roll_reaches_transcription_even_when_speech_ends_before_the_wake_score() {
    let first_words = vec![1200; PRE_ROLL];
    let mut recording = Recording::new(1, true, first_words.clone(), RecordingGain::Fixed(1.0));
    let silence = [0; VAD_SAMPLES];
    for _ in 1..FIRST_SPEECH_TIMEOUT / VAD_SAMPLES {
        assert_eq!(recording.push(&silence), None);
    }
    assert_eq!(recording.push(&silence), Some(true));
    assert_eq!(&recording.samples[..PRE_ROLL], first_words.as_slice());
}

#[test]
fn quiet_speech_ends_at_the_pause_instead_of_the_first_speech_timeout() {
    let mut recording = Recording::new(1, true, Vec::new(), RecordingGain::Fixed(1.0));
    for _ in 0..8 {
        assert_eq!(recording.push(&[128; VAD_SAMPLES]), None);
    }
    for _ in 1..END_SILENCE / VAD_SAMPLES {
        assert_eq!(recording.push(&[12; VAD_SAMPLES]), None);
    }
    assert_eq!(recording.push(&[12; VAD_SAMPLES]), Some(true));
}

#[test]
fn wake_gain_amplifies_detector_input_without_changing_the_recording_pre_roll() {
    let mut detector = Detector::new(None);
    detector.frame = vec![250, -250, 15_000, -15_000];
    detector.buffer_wake_audio(4.0);
    assert_eq!(detector.wake_frame, [1000, -1000, i16::MAX, i16::MIN]);
    let mut pre_roll: Vec<_> = detector.history.into();
    assert_eq!(pre_roll, detector.frame);
    RecordingGain::Fixed(2.0).prepare(&mut pre_roll);
    assert_eq!(pre_roll, [500, -500, 30_000, -30_000]);
}

#[test]
fn automatic_transcription_gain_preserves_the_input_gain_and_bounded_normalization() {
    let mut quiet = [80, -80];
    RecordingGain::Automatic(4.0).prepare(&mut quiet);
    assert_eq!(quiet, [5120, -5120]);
    let mut loud = [20_000, -20_000];
    RecordingGain::Automatic(4.0).prepare(&mut loud);
    assert_eq!(loud, [i16::MAX, i16::MIN]);
}

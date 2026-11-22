use super::{END_SILENCE, FIRST_SPEECH_TIMEOUT, PRE_ROLL, Recording, VAD_SAMPLES};

#[test]
fn pre_roll_reaches_transcription_even_when_speech_ends_before_the_wake_score() {
    let first_words = vec![1200; PRE_ROLL];
    let mut recording = Recording::new(1, true, first_words.clone());
    let silence = [0; VAD_SAMPLES];
    for _ in 1..FIRST_SPEECH_TIMEOUT / VAD_SAMPLES {
        assert_eq!(recording.push(&silence), None);
    }
    assert_eq!(recording.push(&silence), Some(true));
    assert_eq!(&recording.samples[..PRE_ROLL], first_words.as_slice());
}

#[test]
fn quiet_speech_ends_at_the_pause_instead_of_the_first_speech_timeout() {
    let mut recording = Recording::new(1, true, Vec::new());
    for _ in 0..8 {
        assert_eq!(recording.push(&[128; VAD_SAMPLES]), None);
    }
    for _ in 1..END_SILENCE / VAD_SAMPLES {
        assert_eq!(recording.push(&[12; VAD_SAMPLES]), None);
    }
    assert_eq!(recording.push(&[12; VAD_SAMPLES]), Some(true));
}

use super::*;
use crate::wav::PcmAudio;

#[test]
fn listening_eyes_blink_and_the_ring_stays_lit() {
    let open = draw(&Scene::Listening, Duration::ZERO);
    let blink = draw(&Scene::Listening, Duration::from_millis(1700));
    for digit in [0, 3] {
        assert_eq!(open.clock[segment(digit, b'A')], 255);
        assert_eq!(blink.clock[segment(digit, b'A')], 0);
        assert_eq!(blink.clock[segment(digit, b'G')], 255);
        assert_eq!(open.clock[segment(digit, b'D')], 0);
    }
    for digit in [1, 2] {
        assert_eq!(open.clock[segment(digit, b'A')], 0);
    }
    assert!(open.ring.iter().any(|&channel| channel > 0));
    assert_eq!(open.ring, blink.ring);
}

#[test]
fn thinking_moves_the_light_to_the_next_ring_pixel() {
    let first = draw(&Scene::Thinking, Duration::ZERO);
    let next = draw(&Scene::Thinking, Duration::from_millis(192));
    assert!(first.ring[0] > first.ring[3]);
    assert!(next.ring[3] > next.ring[0]);
    assert_eq!(first.clock, next.clock);
}

#[test]
fn mouth_brightness_follows_pcm_and_closes_after_playback() {
    let sample_rate = 16_000;
    let samples_per_frame = (sample_rate * FRAME_MS as u32 / 1_000) as usize;
    let data = [0_i16, 600, 12_000, 12_000, 12_000, 12_000, 0, 0, 0]
        .into_iter()
        .flat_map(|sample| std::iter::repeat_n(sample, samples_per_frame))
        .flat_map(i16::to_le_bytes)
        .collect();
    let mut wave = Vec::new();
    PcmAudio {
        data,
        sample_rate,
        channels: 1,
    }
    .write_wav(&mut wave)
    .unwrap();
    let levels = speech_levels(&wave).unwrap();
    assert_eq!(levels[0], 0);
    assert!(levels[1] > 0 && levels[2] > levels[1]);
    assert!(levels[2] < MAX_MOUTH_LEVEL / 2);
    assert!(levels[2..=5].windows(2).all(|pair| pair[0] < pair[1]));
    assert!(levels[5] > MAX_MOUTH_LEVEL * 3 / 4);
    assert!(levels[5..].windows(2).all(|pair| pair[0] > pair[1]));
    let scene = Scene::Speaking {
        start: Instant::now(),
        levels,
    };
    let loud = draw(&scene, Duration::from_millis(5 * FRAME_MS));
    let ended = draw(&scene, Duration::from_millis(9 * FRAME_MS));
    let mouth = loud.clock[segment(0, b'D')];
    assert!(mouth > 190);
    for digit in 0..4 {
        assert_eq!(loud.clock[segment(digit, b'D')], mouth);
        assert_eq!(ended.clock[segment(digit, b'D')], 0);
        for &letter in b"CE" {
            assert_eq!(loud.clock[segment(digit, letter)], 0);
        }
    }
    assert_eq!(loud.clock[segment(0, b'A')], ended.clock[segment(0, b'A')]);
}

#[test]
fn speech_levels_accept_embedded_announcements_with_extra_wave_chunks() {
    for wave in [
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/error-ru.wav")).as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/no-speech-ru.wav"
        ))
        .as_slice(),
    ] {
        assert!(speech_levels(wave).unwrap().iter().any(|&level| level > 0));
    }
    assert!(speech_levels(b"RIFF").is_none());
}

#[test]
fn speech_envelope_keeps_its_timing_and_smoothing_across_pcm_chunks() {
    let pcm: Vec<_> = (0..5_500)
        .map(|i| (i * 313) as i16)
        .flat_map(i16::to_le_bytes)
        .collect();
    let mut whole = SpeechEnvelope::new(22_050, 4);
    let mut expected = whole.push(&pcm);
    expected.extend(whole.finish());
    let mut stream = SpeechEnvelope::new(22_050, 4);
    let mut actual: Vec<_> = pcm
        .chunks(412)
        .flat_map(|chunk| stream.push(chunk))
        .collect();
    actual.extend(stream.finish());
    assert_eq!(actual, expected);
}

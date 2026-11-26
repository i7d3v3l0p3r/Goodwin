use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Context, Result, ensure};
use goodwin_plugin::provider::SpeechAudio;

pub(crate) fn decode_pcm(audio: &SpeechAudio, byte_limit: usize) -> Result<Vec<u8>> {
    decode(audio, byte_limit, 0)
}

#[cfg(not(target_os = "espidf"))]
pub(crate) fn decode_wav(audio: SpeechAudio) -> Result<Vec<u8>> {
    use goodwin_plugin::{audio::wav_header, provider::MAX_SPEECH_BYTES};
    const HEADER_LEN: usize = 44;
    let mut wav = decode(&audio, MAX_SPEECH_BYTES, HEADER_LEN)?;
    let header = wav_header(audio.sample_rate, audio.channels, wav.len() - HEADER_LEN)?;
    wav[..HEADER_LEN].copy_from_slice(&header);
    Ok(wav)
}

fn decode(audio: &SpeechAudio, byte_limit: usize, header_len: usize) -> Result<Vec<u8>> {
    ensure!(
        (8_000..=96_000).contains(&audio.sample_rate) && (1..=2).contains(&audio.channels),
        "Invalid plugin PCM format"
    );
    ensure!(
        audio.pcm_base64.len() <= byte_limit.div_ceil(3) * 4,
        "Plugin PCM exceeds size limit"
    );
    let mut bytes = vec![0; header_len + base64::decoded_len_estimate(audio.pcm_base64.len())];
    let len = STANDARD
        .decode_slice(&audio.pcm_base64, &mut bytes[header_len..])
        .context("Decode plugin PCM")?;
    ensure!(
        len != 0 && len <= byte_limit && len.is_multiple_of(2 * usize::from(audio.channels)),
        "Invalid plugin PCM length"
    );
    bytes.truncate(header_len + len);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "espidf"))]
    fn wav_and_streamed_pcm_preserve_stereo_frames_and_the_actual_payload_length() {
        let pcm = [1, 2, 3, 4, 5, 6, 7, 8];
        let audio = SpeechAudio {
            pcm_base64: STANDARD.encode(pcm),
            channels: 2,
            sample_rate: 44_100,
        };
        assert_eq!(decode_pcm(&audio, pcm.len()).unwrap(), pcm);
        let wav = decode_wav(audio).unwrap();
        assert_eq!(
            &wav[..44],
            &goodwin_plugin::audio::wav_header(44_100, 2, pcm.len()).unwrap()
        );
        assert_eq!(wav[44..], pcm);
    }

    #[test]
    fn pcm_limits_apply_to_decoded_bytes_and_complete_frames() {
        let mut audio = SpeechAudio {
            pcm_base64: STANDARD.encode([0; 6]),
            channels: 1,
            sample_rate: 16_000,
        };
        // Four and six bytes have the same encoded length.
        assert!(decode_pcm(&audio, 4).is_err());
        audio.channels = 2;
        assert!(decode_pcm(&audio, 6).is_err());
        audio.pcm_base64.clear();
        assert!(decode_pcm(&audio, 6).is_err());
    }
}

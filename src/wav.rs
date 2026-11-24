use std::io;

pub use goodwin_plugin::audio::PcmAudio;

pub fn check_recording(data: &[u8]) -> io::Result<()> {
    if data.len() <= 44 || !data.starts_with(b"RIFF") || &data[8..12] != b"WAVE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The input must be a WAV recording containing audio",
        ));
    }
    Ok(())
}

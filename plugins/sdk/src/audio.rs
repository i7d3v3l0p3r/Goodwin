use std::io::{self, Write};

/// Apply gain and saturate at the signed 16-bit PCM limits.
#[inline]
pub fn scale_sample(sample: i16, gain: f32) -> i16 {
    (f32::from(sample) * gain) as i16
}

pub struct PcmAudio {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmAudio {
    pub fn write_wav(&self, mut target: impl Write) -> io::Result<()> {
        let header = wav_header(self.sample_rate, self.channels, self.data.len())?;
        target.write_all(&header)?;
        target.write_all(&self.data)
    }
}

/// Build the WAV header for signed 16-bit little-endian PCM.
pub fn wav_header(sample_rate: u32, channels: u16, data_len: usize) -> io::Result<[u8; 44]> {
    const BITS_PER_SAMPLE: u16 = 16;
    let block_size = channels
        .checked_mul(BITS_PER_SAMPLE / 8)
        .filter(|size| *size != 0)
        .ok_or_else(|| invalid_pcm("Invalid PCM channel count"))?;
    let byte_rate = sample_rate
        .checked_mul(u32::from(block_size))
        .filter(|rate| *rate != 0)
        .ok_or_else(|| invalid_pcm("Invalid PCM sample rate"))?;
    if data_len == 0 || !data_len.is_multiple_of(usize::from(block_size)) {
        return Err(invalid_pcm(
            "PCM output is empty or contains a partial frame",
        ));
    }
    let data_size = u32::try_from(data_len)
        .ok()
        .filter(|size| *size <= u32::MAX - 36)
        .ok_or_else(|| invalid_pcm("PCM output is too large for a WAV file"))?;

    let mut header = [0; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(data_size + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_size.to_le_bytes());
    header[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_size.to_le_bytes());
    Ok(header)
}

fn invalid_pcm(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

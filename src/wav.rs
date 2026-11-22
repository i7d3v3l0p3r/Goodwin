use std::io::{self, Write};

pub fn check_recording(data: &[u8]) -> io::Result<()> {
    if data.len() <= 44 || !data.starts_with(b"RIFF") || &data[8..12] != b"WAVE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The input must be a WAV recording containing audio",
        ));
    }
    Ok(())
}

pub struct PcmAudio {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmAudio {
    pub fn write_wav(&self, mut target: impl Write) -> io::Result<()> {
        const BITS_PER_SAMPLE: u16 = 16;
        let block_size = self
            .channels
            .checked_mul(BITS_PER_SAMPLE / 8)
            .filter(|size| *size != 0)
            .ok_or_else(|| invalid_pcm("Invalid PCM channel count"))?;
        let byte_rate = self
            .sample_rate
            .checked_mul(u32::from(block_size))
            .filter(|rate| *rate != 0)
            .ok_or_else(|| invalid_pcm("Invalid PCM sample rate"))?;
        if self.data.is_empty() || !self.data.len().is_multiple_of(usize::from(block_size)) {
            return Err(invalid_pcm(
                "PCM output is empty or contains a partial frame",
            ));
        }
        let data_size = u32::try_from(self.data.len())
            .ok()
            .filter(|size| *size <= u32::MAX - 36)
            .ok_or_else(|| invalid_pcm("PCM output is too large for a WAV file"))?;

        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(data_size + 36).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16_u32.to_le_bytes());
        header.extend_from_slice(&1_u16.to_le_bytes());
        header.extend_from_slice(&self.channels.to_le_bytes());
        header.extend_from_slice(&self.sample_rate.to_le_bytes());
        header.extend_from_slice(&byte_rate.to_le_bytes());
        header.extend_from_slice(&block_size.to_le_bytes());
        header.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data_size.to_le_bytes());

        target.write_all(&header)?;
        target.write_all(&self.data)
    }
}

fn invalid_pcm(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

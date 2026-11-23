pub(super) fn scale_sample(sample: i16, gain: f32) -> i16 {
    // Float-to-integer casts saturate at the PCM limits.
    (f32::from(sample) * gain) as i16
}

#[derive(Clone, Copy)]
pub(crate) enum RecordingGain {
    Automatic(f32),
    Fixed(f32),
}

impl RecordingGain {
    pub(super) fn new(fixed: Option<f32>, fallback: f32) -> Self {
        fixed.map_or(Self::Automatic(fallback), Self::Fixed)
    }

    pub(super) fn input_gain(self) -> f32 {
        match self {
            Self::Automatic(gain) | Self::Fixed(gain) => gain,
        }
    }

    pub(crate) fn prepare(self, samples: &mut [i16]) {
        let input_gain = self.input_gain();
        for sample in samples.iter_mut() {
            *sample = scale_sample(*sample, input_gain);
        }
        if let Self::Automatic(_) = self {
            let peak = samples
                .iter()
                .map(|sample| sample.unsigned_abs())
                .max()
                .unwrap_or(0);
            let gain = (16_384.0 / f32::from(peak.max(1))).clamp(1.0, 16.0);
            if gain > 1.0 {
                for sample in samples {
                    *sample = scale_sample(*sample, gain);
                }
            }
        }
    }
}

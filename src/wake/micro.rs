use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use ort::{
    session::{Session, SessionInputValue, builder::GraphOptimizationLevel},
    tensor::TensorElementType,
    value::{DynValue, Tensor, TensorRef, ValueType},
};
use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use super::{FRAME_SAMPLES, SAMPLE_RATE};

const FFT_SIZE: usize = 512;
const HOP: usize = 160;
const WINDOW: usize = 400;
const BANDS: usize = 40;
const FRAMES: usize = FRAME_SAMPLES / HOP;
const TAIL: usize = FFT_SIZE - HOP;
const WARMUP_FRAMES: usize = 2 * SAMPLE_RATE / FRAME_SAMPLES;
type Features = [f32; FRAMES * BANDS];

pub(super) struct MicroModel {
    session: Session,
    frontend: Frontend,
    input_names: Vec<String>,
    state_outputs: Vec<String>,
    state: Vec<DynValue>,
    warm: Vec<Vec<f32>>,
}

impl MicroModel {
    pub(super) fn load(bytes: &[u8], params: [f32; 3]) -> Result<Self> {
        // Match the backend used by LiveKit's Rust client on the Station and desktop.
        ort::set_api(ort_tract::api());
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .commit_from_memory(bytes)?;
        let mut input_names = Vec::new();
        let mut state_outputs = Vec::new();
        let mut state = Vec::new();
        for input in session.inputs() {
            let name = input.name();
            let ValueType::Tensor { ty, shape, .. } = input.dtype() else {
                anyhow::bail!("Micro input {name} must be a tensor");
            };
            ensure!(
                *ty == TensorElementType::Float32,
                "Micro input {name} must be float32"
            );
            match (name, &shape[..]) {
                ("mel", [1, 8, 40]) => continue,
                ("mel", _) => anyhow::bail!("Micro mel input must have shape [1, 8, 40]"),
                (_, [1, length]) if *length > 0 => {
                    state.push(
                        Tensor::from_array((
                            [1, *length as usize],
                            vec![0.0_f32; *length as usize],
                        ))?
                        .into_dyn(),
                    );
                }
                _ => anyhow::bail!("Invalid Micro state input {name}"),
            }
            input_names.push(name.to_owned());
            state_outputs.push(format!("{name}_out"));
        }
        let has_output = |name| session.outputs().iter().any(|output| output.name() == name);
        ensure!(
            has_output("score") && state_outputs.iter().all(|name| has_output(name)),
            "Micro model must return score and all recurrent states"
        );
        let mut model = Self {
            session,
            frontend: Frontend::new(params),
            input_names,
            state_outputs,
            state,
            warm: Vec::new(),
        };
        for _ in 0..WARMUP_FRAMES {
            model.step(&[0; FRAME_SAMPLES])?;
        }
        model.warm = model
            .state
            .iter()
            .map(|state| {
                let (_, values) = state.try_extract_tensor::<f32>()?;
                Ok(values.to_vec())
            })
            .collect::<Result<_>>()?;
        Ok(model)
    }

    pub(super) fn reset(&mut self) -> Result<()> {
        self.frontend.pcm.fill(0.0);
        self.state = self
            .warm
            .iter()
            .map(|values| Ok(Tensor::from_array(([1, values.len()], values.clone()))?.into_dyn()))
            .collect::<Result<_>>()?;
        Ok(())
    }

    pub(super) fn step(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let features = self.frontend.process(frame);
        let input = TensorRef::from_array_view(([1, FRAMES, BANDS], features.as_slice()))?;
        let mut inputs = Vec::with_capacity(self.state.len() + 1);
        inputs.push(("mel", SessionInputValue::from(input)));
        inputs.extend(
            self.input_names
                .iter()
                .map(String::as_str)
                .zip(self.state.iter().map(SessionInputValue::from)),
        );
        let mut outputs = self.session.run(inputs)?;
        let (_, scores) = outputs
            .get("score")
            .context("Micro score missing")?
            .try_extract_tensor::<f32>()?;
        let [score] = scores else {
            anyhow::bail!("Micro model returned more or fewer than one score");
        };
        let score = *score;
        ensure!(score.is_finite(), "Micro model returned an invalid score");
        let next = self
            .state_outputs
            .iter()
            .map(|name| {
                outputs
                    .remove(name)
                    .with_context(|| format!("Micro state {name} missing"))
            })
            .collect::<Result<_>>()?;
        drop(outputs);
        self.state = next;
        Ok(score)
    }
}

struct MelFilter {
    first: usize,
    weights: Vec<f32>,
}

struct Frontend {
    pcm: [f32; TAIL + FRAME_SAMPLES],
    window: [f32; WINDOW],
    filters: [MelFilter; BANDS],
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    params: [f32; 3],
}

impl Frontend {
    fn new(params: [f32; 3]) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex32::ZERO; fft.get_inplace_scratch_len()];
        Self {
            pcm: [0.0; TAIL + FRAME_SAMPLES],
            window: std::array::from_fn(|i| {
                let angle = std::f64::consts::TAU * (i + 1) as f64 / (WINDOW + 1) as f64;
                (0.5 - 0.5 * angle.cos()) as f32
            }),
            filters: mel_filters(),
            fft,
            scratch,
            params,
        }
    }

    fn process(&mut self, input: &[i16; FRAME_SAMPLES]) -> Features {
        for (dst, &sample) in self.pcm[TAIL..].iter_mut().zip(input) {
            *dst = f32::from(sample) / 32768.0;
        }
        let mut features = [0.0; FRAMES * BANDS];
        let [floor, offset, scale] = self.params;
        for (i, frame) in features.as_chunks_mut::<BANDS>().0.iter_mut().enumerate() {
            // Right-align the 25 ms Hann window so the 10 ms step has no lookahead.
            let start = i * HOP + FFT_SIZE - WINDOW;
            let mut spectrum = [Complex32::ZERO; FFT_SIZE];
            for ((dst, sample), gain) in spectrum[FFT_SIZE - WINDOW..]
                .iter_mut()
                .zip(&self.pcm[start..start + WINDOW])
                .zip(&self.window)
            {
                dst.re = sample * gain;
            }
            self.fft
                .process_with_scratch(&mut spectrum, &mut self.scratch);
            let power: [f32; FFT_SIZE / 2 + 1] = std::array::from_fn(|k| spectrum[k].norm_sqr());
            for (dst, filter) in frame.iter_mut().zip(&self.filters) {
                let energy = power[filter.first..]
                    .iter()
                    .zip(&filter.weights)
                    .map(|(value, weight)| value * weight)
                    .sum::<f32>();
                *dst = ((energy + floor).ln() + offset) / scale;
            }
        }
        self.pcm.copy_within(FRAME_SAMPLES.., 0);
        features
    }
}

fn mel_filters() -> [MelFilter; BANDS] {
    // Librosa's Slaney mel scale and area normalization, as used by LiveKit v2.
    let linear_step = 200.0 / 3.0;
    let log_step = 6.4_f64.ln() / 27.0;
    let min_mel = 40.0 / linear_step;
    let max_mel = 15.0 + (7600.0_f64 / 1000.0).ln() / log_step;
    let edges: [f64; BANDS + 2] = std::array::from_fn(|i| {
        let mel = min_mel + (max_mel - min_mel) * i as f64 / (BANDS + 1) as f64;
        if mel < 15.0 {
            mel * linear_step
        } else {
            1000.0 * ((mel - 15.0) * log_step).exp()
        }
    });
    let bin_hz = SAMPLE_RATE as f64 / FFT_SIZE as f64;
    std::array::from_fn(|i| {
        let [left, center, right] = [edges[i], edges[i + 1], edges[i + 2]];
        let first = (left / bin_hz).ceil() as usize;
        let last = (right / bin_hz).floor() as usize;
        let weights = (first..=last)
            .map(|bin| {
                let hz = bin as f64 * bin_hz;
                let slope = ((hz - left) / (center - left)).min((right - hz) / (right - center));
                (2.0 * slope / (right - left)) as f32
            })
            .collect();
        MelFilter { first, weights }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_matches_livekit_across_pcm_blocks() {
        let mut frontend = Frontend::new([1e-9, 9.0, 6.0]);
        let bins = [0, 1, 15, 39, 41, 158, 209, 319];
        // LiveKit v2.1.3 host_mel on the concatenated samples.
        let expected = [
            [
                0.557074, 0.565125, 0.711879, 0.860951, 0.503218, 0.950652, 0.676320, 1.096269,
            ],
            [
                -0.410562, 0.359906, 0.225345, 1.09627, 0.357402, 0.950649, 0.676716, 1.096272,
            ],
        ];
        for (block, expected) in expected.into_iter().enumerate() {
            let input = std::array::from_fn(|i| {
                (((block * FRAME_SAMPLES + i) * 7919) % 16384) as i16 - 8192
            });
            let output = frontend.process(&input);
            for (index, expected) in bins.into_iter().zip(expected) {
                assert!(
                    (output[index] - expected).abs() < 0.00005,
                    "block {block}, bin {index}"
                );
            }
        }
    }
}

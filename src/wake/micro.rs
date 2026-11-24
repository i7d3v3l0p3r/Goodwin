use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use tract_onnx::prelude::{
    DatumType, Framework, InferenceModelExt, IntoTValue, IntoTensor, Tensor, TypedModel,
    TypedRunnableModel,
};

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
    plan: TypedRunnableModel<TypedModel>,
    frontend: Frontend,
    state: Vec<Tensor>,
    warm: Vec<Tensor>,
}

impl MicroModel {
    pub(super) fn load(mut bytes: &[u8], params: [f32; 3]) -> Result<Self> {
        let onnx = tract_onnx::onnx();
        let proto = onnx.proto_model_for_read(&mut bytes)?;
        let graph = proto.graph.as_ref().context("Micro model has no graph")?;
        let state_names: Vec<_> = graph
            .input
            .iter()
            .map(|input| input.name.as_str())
            .filter(|&name| name != "mel")
            .collect();
        let state_outputs: Vec<_> = state_names
            .iter()
            .map(|name| format!("{name}_out"))
            .collect();
        let output_names = std::iter::once("score").chain(state_outputs.iter().map(String::as_str));
        ensure!(
            output_names
                .clone()
                .all(|name| graph.output.iter().any(|output| output.name == name)),
            "Micro model must return score and all recurrent states"
        );
        let input_names = std::iter::once("mel").chain(state_names.iter().copied());
        let mut graph = onnx.model_for_proto_model(&proto)?;
        graph.set_input_names(input_names)?;
        graph.set_output_names(output_names)?;
        let graph = graph.into_optimized()?;

        let mel = graph.input_fact(0)?;
        ensure!(
            mel.datum_type == DatumType::F32
                && mel.shape.as_concrete() == Some(&[1, FRAMES, BANDS]),
            "Micro mel input must be float32 with shape [1, {FRAMES}, {BANDS}]"
        );
        let mut state = Vec::with_capacity(state_names.len());
        for (index, name) in state_names.iter().enumerate() {
            let input = graph.input_fact(index + 1)?;
            let Some([1, length]) = input.shape.as_concrete() else {
                anyhow::bail!("Invalid Micro state input {name}");
            };
            ensure!(
                input.datum_type == DatumType::F32 && *length > 0,
                "Invalid Micro state input {name}"
            );
            state.push(Tensor::zero::<f32>(&[1, *length])?);
        }
        let mut model = Self {
            plan: graph.into_runnable()?,
            frontend: Frontend::new(params),
            state,
            warm: Vec::new(),
        };
        for _ in 0..WARMUP_FRAMES {
            model.step(&[0; FRAME_SAMPLES])?;
        }
        model.warm.clone_from(&model.state);
        Ok(model)
    }

    pub(super) fn reset(&mut self) {
        self.frontend.pcm.fill(0.0);
        self.state.clone_from(&self.warm);
    }

    pub(super) fn step(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let features = self.frontend.process(frame);
        let mel = Tensor::from_shape(&[1, FRAMES, BANDS], &features)?.into_tvalue();
        // Move the tensors into the plan so its operators can reuse exclusive inputs.
        let inputs = std::iter::once(mel)
            .chain(self.state.drain(..).map(IntoTValue::into_tvalue))
            .collect();
        let mut outputs = self.plan.run(inputs)?.into_iter();
        let score = outputs.next().context("Micro score missing")?;
        let &[score] = score.as_slice::<f32>()? else {
            anyhow::bail!("Micro model returned more or fewer than one score");
        };
        ensure!(score.is_finite(), "Micro model returned an invalid score");
        self.state.extend(outputs.map(IntoTensor::into_tensor));
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
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[test]
    fn recurrent_states_follow_tensor_names_and_reset_to_warmup() {
        // The graph declares inputs as s1, mel, s0 and outputs as s0_out, s1_out, score.
        // On silence, score = s0, s0_out = s0 + mean(s1), and s1_out = s1 + [1, 3].
        let encoded = concat!(
            "CAg6igMKOQoDbWVsEgZzaWduYWwiClJlZHVjZU1lYW4qDQoEYXhlc0ABQAKgAQcqDwoIa2Vl",
            "cGRpbXMYAKABAgoYCgJzMAoGc2lnbmFsEgVzY29yZSIDQWRkCjgKAnMxEghmZWVkYmFjayIK",
            "UmVkdWNlTWVhbioLCgRheGVzQAGgAQcqDwoIa2VlcGRpbXMYAaABAgobCgJzMAoIZmVlZGJh",
            "Y2sSBnMwX291dCIDQWRkChwKAnMxCglpbmNyZW1lbnQSBnMxX291dCIDQWRkEg1vcmRlcmVk",
            "LXN0YXRlKhsIAQgCEAEiCAAAgD8AAEBAQglpbmNyZW1lbnRaFAoCczESDgoMCAESCAoCCAEK",
            "AggCWhkKA21lbBISChAIARIMCgIIAQoCCAgKAggoWhQKAnMwEg4KDAgBEggKAggBCgIIAWIY",
            "CgZzMF9vdXQSDgoMCAESCAoCCAEKAggBYhgKBnMxX291dBIOCgwIARIICgIIAQoCCAJiFwoF",
            "c2NvcmUSDgoMCAESCAoCCAEKAggBQgQKABAN",
        );
        let bytes = STANDARD.decode(encoded).unwrap();
        let mut model = MicroModel::load(&bytes, [1.0, 0.0, 1.0]).unwrap();
        let silence = [0; FRAME_SAMPLES];
        assert_eq!(model.step(&silence).unwrap(), 600.0);
        assert_eq!(model.step(&silence).unwrap(), 650.0);
        model.reset();
        assert_eq!(model.step(&silence).unwrap(), 600.0);
    }

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

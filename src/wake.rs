use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use livekit_wakeword::WakeWordModel;
use prost::Message;

mod catalog;
mod micro;

pub(crate) use catalog::{InstallError, PendingModel, WakeModels};
pub use livekit_wakeword::{FRAME_SAMPLES, SAMPLE_RATE};

pub struct WakeModel(Backend);

enum Backend {
    Standard(Box<WakeWordModel>),
    Micro(Box<micro::MicroModel>),
}

impl WakeModel {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Read wake model {}", path.display()))?;
        let summary = OnnxSummary::decode(bytes.as_slice()).context("Decode ONNX model")?;
        let graph = summary.graph.as_ref().context("ONNX model has no graph")?;
        let has_input = |name| graph.inputs.iter().any(|input| input.name == name);
        match (has_input("chunk"), has_input("mel")) {
            (true, false) => {
                drop(bytes);
                Ok(Self(Backend::Standard(Box::new(WakeWordModel::new(
                    &[path],
                    SAMPLE_RATE as u32,
                )?))))
            }
            (false, true) => {
                let params = summary.micro_params()?;
                Ok(Self(Backend::Micro(Box::new(micro::MicroModel::load(
                    &bytes, params,
                )?))))
            }
            _ => bail!("Expected a LiveKit v2 Standard or Micro ONNX graph"),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self.0 {
            Backend::Standard(_) => "LiveKit Standard",
            Backend::Micro(_) => "LiveKit Micro",
        }
    }

    pub fn predict(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let score = match &mut self.0 {
            Backend::Standard(model) => model
                .predict(frame)?
                .into_values()
                .next()
                .context("Standard wake model returned no score"),
            Backend::Micro(model) => model.step(frame),
        }?;
        ensure!(score.is_finite(), "Wake model returned an invalid score");
        Ok(score)
    }

    pub fn reset(&mut self) -> Result<()> {
        match &mut self.0 {
            Backend::Standard(model) => Ok(model.reset()?),
            Backend::Micro(model) => model.reset(),
        }
    }
}

// Only the graph input names and custom properties from the ONNX protobuf.
// Prost skips nodes and initializer weights until the runtime loads the full graph.
#[derive(Message)]
struct OnnxSummary {
    #[prost(message, optional, tag = "7")]
    graph: Option<GraphInputs>,
    #[prost(message, repeated, tag = "14")]
    properties: Vec<ModelProperty>,
}

impl OnnxSummary {
    fn micro_params(&self) -> Result<[f32; 3]> {
        let Some(property) = self
            .properties
            .iter()
            .find(|property| property.key == "livekit_wakeword.mel_params")
        else {
            return Ok([1e-6, 6.0, 4.0]);
        };
        let params: [f32; 3] =
            serde_json::from_str(&property.value).context("Invalid Micro log-mel metadata")?;
        ensure!(
            params.iter().all(|value| value.is_finite()) && params[0] > 0.0 && params[2] > 0.0,
            "Micro log-mel metadata must have a positive floor and scale"
        );
        Ok(params)
    }
}

#[derive(Message)]
struct GraphInputs {
    #[prost(message, repeated, tag = "11")]
    inputs: Vec<GraphInput>,
}

#[derive(Message)]
struct GraphInput {
    #[prost(string, tag = "1")]
    name: String,
}

#[derive(Message)]
struct ModelProperty {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, tag = "2")]
    value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct ReferenceClip {
        model: String,
        pcm: Vec<i16>,
        scores: Vec<f32>,
    }

    #[test]
    #[ignore = "uses the locally trained ONNX files and reference inference traces"]
    fn trained_models_match_livekit_reference_and_reset() {
        let path = std::env::var_os("GOODWIN_WAKE_REFERENCE").expect("set GOODWIN_WAKE_REFERENCE");
        let clips: Vec<ReferenceClip> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(!clips.is_empty());
        for clip in clips {
            let mut model = WakeModel::load(Path::new(&clip.model)).unwrap();
            assert_eq!(clip.pcm.len(), clip.scores.len() * FRAME_SAMPLES);
            for _ in 0..2 {
                for (frame, expected) in clip
                    .pcm
                    .as_chunks::<FRAME_SAMPLES>()
                    .0
                    .iter()
                    .zip(&clip.scores)
                {
                    let score = model.predict(frame).unwrap();
                    assert!(
                        (score - expected).abs() < 0.001,
                        "{}: {score} differs from {expected}",
                        model.kind()
                    );
                    assert_eq!(score >= 0.5, *expected >= 0.5);
                }
                model.reset().unwrap();
            }
        }
    }
}

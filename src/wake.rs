use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use livekit_wakeword::WakeWordModel;
use prost::Message;
use sha2::{Digest, Sha256};

use crate::wake_catalog::{Runtime, Selection, tflite_marker};

mod catalog;
mod micro;
#[cfg(feature = "wake-tflite")]
use crate::wake_tflite::TfliteModel;

pub(crate) use catalog::{InstallError, PendingModel, WakeModels};
pub use livekit_wakeword::{FRAME_SAMPLES, SAMPLE_RATE};

pub(crate) fn runtimes() -> &'static [Runtime] {
    let with_tflite = cfg!(feature = "wake-tflite");
    if with_tflite {
        // The desktop installer and package builder read the compiled capability.
        std::hint::black_box(tflite_marker());
    }
    Runtime::available(with_tflite)
}

pub struct WakeModel {
    backend: Backend,
    sha256: String,
    calibrated_threshold: Option<f32>,
}

enum Backend {
    Standard(Box<WakeWordModel>),
    Micro(Box<micro::MicroModel>),
    #[cfg(feature = "wake-tflite")]
    Tflite(TfliteModel),
}

impl WakeModel {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_file(path, None)
    }

    pub(crate) fn load_selected(path: &Path, selection: &Selection) -> Result<Self> {
        ensure!(
            runtimes().contains(&selection.runtime),
            "The selected wake runtime is unavailable in this build"
        );
        Self::load_file(path, Some(selection))
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    fn load_file(path: &Path, selection: Option<&Selection>) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Read wake model {}", path.display()))?;
        let sha256 = crate::checksum::hex(&Sha256::digest(&bytes));
        let tflite = bytes.get(4..8) == Some(b"TFL3");
        if let Some(selection) = selection {
            ensure!(
                sha256 == selection.sha256 && tflite == (selection.runtime == Runtime::MicroTflite),
                "Wake model does not match its selection"
            );
        }
        if tflite {
            #[cfg(feature = "wake-tflite")]
            return Ok(Self {
                backend: Backend::Tflite(TfliteModel::load(bytes)?),
                sha256,
                calibrated_threshold: Some(
                    selection
                        .and_then(|entry| entry.score_threshold)
                        .unwrap_or(0.5),
                ),
            });
            #[cfg(not(feature = "wake-tflite"))]
            bail!("This Goodwin build does not support LiveKit Micro TFLite");
        }
        let summary = OnnxSummary::decode(bytes.as_slice()).context("Decode ONNX model")?;
        let graph = summary.graph.as_ref().context("ONNX model has no graph")?;
        let has_input = |name| graph.inputs.iter().any(|input| input.name == name);
        let backend = match (has_input("chunk"), has_input("mel")) {
            (true, false) => {
                drop(bytes);
                Backend::Standard(Box::new(WakeWordModel::new(&[path], SAMPLE_RATE as u32)?))
            }
            (false, true) => {
                let params = summary.micro_params()?;
                Backend::Micro(Box::new(micro::MicroModel::load(&bytes, params)?))
            }
            _ => bail!("Expected a LiveKit v2 Standard or Micro ONNX graph"),
        };
        let model = Self {
            backend,
            sha256,
            calibrated_threshold: None,
        };
        ensure!(
            selection.is_none_or(|selection| model.runtime() == selection.runtime),
            "Wake model does not match its selection"
        );
        Ok(model)
    }

    fn runtime(&self) -> Runtime {
        match self.backend {
            Backend::Standard(_) => Runtime::Standard,
            Backend::Micro(_) => Runtime::Micro,
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(_) => Runtime::MicroTflite,
        }
    }

    pub(crate) fn threshold(&self, sensitivity: f32) -> f32 {
        self.calibrated_threshold.map_or_else(
            || 0.85 - 0.55 * sensitivity,
            |center| (center + 0.55 * (0.5 - sensitivity)).clamp(0.0, 1.0),
        )
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn kind(&self) -> &'static str {
        match self.backend {
            Backend::Standard(_) => "LiveKit Standard",
            Backend::Micro(_) => "LiveKit Micro",
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(_) => "LiveKit Micro INT8",
        }
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    pub fn predict(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let score = match &mut self.backend {
            Backend::Standard(model) => model
                .predict(frame)?
                .into_values()
                .next()
                .context("Standard wake model returned no score"),
            Backend::Micro(model) => model.step(frame),
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(model) => model.predict(frame),
        }?;
        ensure!(score.is_finite(), "Wake model returned an invalid score");
        Ok(score)
    }

    pub fn reset(&mut self) -> Result<()> {
        match &mut self.backend {
            Backend::Standard(model) => Ok(model.reset()?),
            Backend::Micro(model) => {
                model.reset();
                Ok(())
            }
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(model) => {
                model.reset();
                Ok(())
            }
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
    fn invalid_tflite_does_not_load_as_onnx() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.tflite");
        fs::write(&path, b"\x08\x00\x00\x00TFL3").unwrap();
        let error = WakeModel::load(&path).err().unwrap().to_string();
        assert!(error.contains("TFLite"));
    }

    #[cfg(feature = "wake-tflite")]
    #[test]
    #[ignore = "uses the published INT8 model"]
    fn tflite_reset_restores_the_warmed_state_and_uses_the_catalog_threshold() {
        let path = std::env::var_os("GOODWIN_TFLITE_MODEL").expect("set GOODWIN_TFLITE_MODEL");
        let path = Path::new(&path);
        let bytes = fs::read(path).unwrap();
        let selection = Selection {
            id: "goodwin-micro-int8".into(),
            wake_word: "Гудвин".into(),
            tier: "micro".into(),
            sha256: crate::checksum::hex(&Sha256::digest(&bytes)),
            runtime: Runtime::MicroTflite,
            score_threshold: Some(0.390625),
        };
        let mut model = WakeModel::load_selected(path, &selection).unwrap();
        assert_eq!(model.threshold(0.5), 0.390625);
        let pcm = std::array::from_fn(|index| (index as i16).wrapping_mul(251));
        let scores = [model.predict(&pcm).unwrap(), model.predict(&pcm).unwrap()];
        model.reset().unwrap();
        assert_eq!(model.predict(&pcm).unwrap(), scores[0]);
        assert_eq!(model.predict(&pcm).unwrap(), scores[1]);
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

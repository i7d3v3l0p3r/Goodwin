use std::{fs, path::Path};

use goodwin_error::{Context, Result, bail, ensure};
#[cfg(feature = "wake-onnx")]
use livekit_wakeword::WakeWordModel;
#[cfg(feature = "wake-onnx")]
use prost::Message;
use sha2::{Digest, Sha256};

use crate::wake_catalog::{INT8_ONLY_MARKER, Runtime, Selection, tflite_marker};

mod catalog;
#[cfg(feature = "wake-onnx")]
mod micro;
#[cfg(feature = "wake-tflite")]
use crate::wake_tflite::TfliteModel;

#[cfg(not(feature = "wake-onnx"))]
pub(crate) use crate::wake_tflite::FRAME_SAMPLES;
pub(crate) use catalog::{InstallError, PendingModel, WakeModels};
#[cfg(feature = "wake-onnx")]
pub use livekit_wakeword::{FRAME_SAMPLES, SAMPLE_RATE};
#[cfg(not(feature = "wake-onnx"))]
pub(crate) const SAMPLE_RATE: usize = crate::voice_core::CAPTURE_HZ as usize;

pub(crate) fn runtimes() -> &'static [Runtime] {
    let with_tflite = cfg!(feature = "wake-tflite");
    let with_onnx = cfg!(feature = "wake-onnx");
    if with_tflite {
        // The desktop installer and package builder read the compiled capability.
        std::hint::black_box(tflite_marker());
    }
    if !with_onnx {
        std::hint::black_box(INT8_ONLY_MARKER);
    }
    Runtime::available(with_onnx, with_tflite)
}

pub struct WakeModel {
    backend: Backend,
    sha256: String,
    calibrated_threshold: Option<f32>,
}

enum Backend {
    #[cfg(feature = "wake-onnx")]
    Standard(Box<WakeWordModel>),
    #[cfg(feature = "wake-onnx")]
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
        #[cfg(feature = "wake-onnx")]
        {
            Self::load_onnx(path, selection, bytes, sha256)
        }
        #[cfg(not(feature = "wake-onnx"))]
        {
            bail!("This Goodwin build supports only Micro INT8 TFLite wake models")
        }
    }

    #[cfg(feature = "wake-onnx")]
    fn load_onnx(
        path: &Path,
        selection: Option<&Selection>,
        bytes: Vec<u8>,
        sha256: String,
    ) -> Result<Self> {
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

    #[cfg(feature = "wake-onnx")]
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
            #[cfg(feature = "wake-onnx")]
            Backend::Standard(_) => "LiveKit Standard",
            #[cfg(feature = "wake-onnx")]
            Backend::Micro(_) => "LiveKit Micro",
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(_) => "LiveKit Micro INT8",
        }
    }

    #[cfg(all(
        target_os = "linux",
        feature = "wake-bench",
        feature = "wake-tflite",
        feature = "hotpath-alloc"
    ))]
    pub(crate) fn tflite_buffers(&mut self) -> Option<[usize; 3]> {
        match &mut self.backend {
            Backend::Tflite(model) => Some(model.buffers()),
            #[cfg(feature = "wake-onnx")]
            _ => None,
        }
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    pub fn predict(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let score = match &mut self.backend {
            #[cfg(feature = "wake-onnx")]
            Backend::Standard(model) => model
                .predict(frame)?
                .into_values()
                .next()
                .context("Standard wake model returned no score"),
            #[cfg(feature = "wake-onnx")]
            Backend::Micro(model) => model.step(frame),
            #[cfg(feature = "wake-tflite")]
            Backend::Tflite(model) => model.predict(frame),
        }?;
        ensure!(score.is_finite(), "Wake model returned an invalid score");
        Ok(score)
    }

    pub fn reset(&mut self) -> Result<()> {
        match &mut self.backend {
            #[cfg(feature = "wake-onnx")]
            Backend::Standard(model) => Ok(model.reset()?),
            #[cfg(feature = "wake-onnx")]
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
#[cfg(feature = "wake-onnx")]
#[derive(Message)]
struct OnnxSummary {
    #[prost(message, optional, tag = "7")]
    graph: Option<GraphInputs>,
    #[prost(message, repeated, tag = "14")]
    properties: Vec<ModelProperty>,
}

#[cfg(feature = "wake-onnx")]
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
            goodwin_json::from_str(&property.value).context("Invalid Micro log-mel metadata")?;
        ensure!(
            params.iter().all(|value| value.is_finite()) && params[0] > 0.0 && params[2] > 0.0,
            "Micro log-mel metadata must have a positive floor and scale"
        );
        Ok(params)
    }
}

#[cfg(feature = "wake-onnx")]
#[derive(Message)]
struct GraphInputs {
    #[prost(message, repeated, tag = "11")]
    inputs: Vec<GraphInput>,
}

#[cfg(feature = "wake-onnx")]
#[derive(Message)]
struct GraphInput {
    #[prost(string, tag = "1")]
    name: String,
}

#[cfg(feature = "wake-onnx")]
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

    #[cfg(feature = "wake-onnx")]
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

    #[cfg(not(feature = "wake-onnx"))]
    #[test]
    fn int8_build_rejects_a_legacy_onnx_selection() {
        assert_eq!(runtimes(), &[Runtime::MicroTflite]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wake-model.onnx");
        fs::write(&path, b"onnx").unwrap();
        let error = WakeModel::load(&path).err().unwrap().to_string();
        assert!(error.contains("only Micro INT8 TFLite"));
    }

    #[cfg(feature = "wake-tflite")]
    #[test]
    #[ignore = "uses pinned public models from scripts/prepare-wake-reference.py"]
    fn published_models_match_the_synthetic_reference() {
        #[derive(Deserialize)]
        struct Fixture {
            pcm_sha256: String,
            models: Vec<ModelFixture>,
        }

        #[derive(Deserialize)]
        struct ModelFixture {
            filename: String,
            #[serde(flatten)]
            selection: Selection,
            assistant_threshold: f32,
            score_tolerance: f32,
            scores: Vec<f32>,
        }

        let fixture: Fixture =
            serde_json::from_str(include_str!("../tests/fixtures/wake_reference.json")).unwrap();
        let directory =
            std::env::var_os("GOODWIN_WAKE_REFERENCE_DIR").expect("set GOODWIN_WAKE_REFERENCE_DIR");
        let directory = Path::new(&directory);
        let mut random = 0x12ab_34cdu32;
        let frames: [[i16; FRAME_SAMPLES]; 16] = std::array::from_fn(|frame| {
            std::array::from_fn(|index| match frame {
                0..=2 | 12.. => 0,
                3..=6 => {
                    let phase = ((frame * FRAME_SAMPLES + index) % 72) as i16;
                    (36 - (phase - 36).abs()) * 220 - 3960
                }
                7..=10 => {
                    random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (random >> 20) as i16 - 2048
                }
                _ if index == FRAME_SAMPLES / 2 => i16::MAX,
                _ => 0,
            })
        });
        let mut digest = Sha256::new();
        for samples in &frames {
            for sample in samples {
                digest.update(sample.to_le_bytes());
            }
        }
        assert_eq!(crate::checksum::hex(&digest.finalize()), fixture.pcm_sha256);
        let mut checked = Vec::new();
        for reference in fixture.models {
            if !runtimes().contains(&reference.selection.runtime) {
                continue;
            }
            let path = directory.join(reference.filename);
            let selection = &reference.selection;
            checked.push(selection.runtime);
            let mut model = WakeModel::load_selected(&path, selection).unwrap();
            let catalog_threshold = selection.score_threshold.unwrap();
            let assistant_threshold = model.threshold(0.5);
            assert!((assistant_threshold - reference.assistant_threshold).abs() < 0.000_001);
            assert_eq!(reference.scores.len(), frames.len());
            for _ in 0..2 {
                for (index, (frame, &expected)) in frames.iter().zip(&reference.scores).enumerate()
                {
                    let score = model.predict(frame).unwrap();
                    assert!(
                        (score - expected).abs() < reference.score_tolerance,
                        "{} frame {index}: {score} differs from {expected}",
                        selection.id
                    );
                    for threshold in [catalog_threshold, assistant_threshold] {
                        assert_eq!(score >= threshold, expected >= threshold);
                    }
                }
                model.reset().unwrap();
            }
        }
        for runtime in runtimes() {
            assert!(
                checked.contains(runtime),
                "Missing reference for {runtime:?}"
            );
        }
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
        std::thread::spawn(move || {
            model.reset().unwrap();
            assert_eq!(model.predict(&pcm).unwrap(), scores[0]);
            assert_eq!(model.predict(&pcm).unwrap(), scores[1]);
        })
        .join()
        .unwrap();
    }

    #[cfg(feature = "wake-onnx")]
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

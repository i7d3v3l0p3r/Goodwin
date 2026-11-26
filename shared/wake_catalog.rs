use std::collections::HashSet;
#[cfg(not(target_os = "espidf"))]
use std::{
    io::{Read, Write},
    time::Duration,
};

#[cfg(not(target_os = "espidf"))]
use goodwin_error::Context;
use goodwin_error::{Result, ensure};
#[cfg(not(target_os = "espidf"))]
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "espidf", test))]
use sha2::{Digest, Sha256};

#[cfg(not(target_os = "espidf"))]
use crate::checksum::Sha256Writer;

pub(crate) const RELEASES: &str = "https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases";
pub(crate) const CATALOG_LIMIT: u64 = 64 * 1024;
const MODEL_LIMIT: u64 = 32 * 1024 * 1024;
#[cfg(not(target_os = "espidf"))]
pub(crate) fn tflite_marker() -> &'static [u8] {
    include_str!("wake-tflite-capability.txt")
        .trim_end()
        .as_bytes()
}

#[cfg(not(target_os = "espidf"))]
pub(crate) const INT8_ONLY_MARKER: &[u8] = b"goodwin.wake.int8-only.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum Runtime {
    #[serde(rename = "livekit-v2-standard-onnx")]
    Standard,
    #[serde(rename = "livekit-v2-micro-onnx")]
    Micro,
    #[serde(rename = "livekit-v2-micro-tflite")]
    MicroTflite,
    #[serde(rename = "microwakeword-tflite")]
    MicroWakeWord,
    #[serde(other)]
    Unknown,
}

impl Runtime {
    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn available(with_onnx: bool, with_tflite: bool) -> &'static [Self] {
        match (with_onnx, with_tflite) {
            (true, true) => &[Self::Standard, Self::Micro, Self::MicroTflite],
            (true, false) => &[Self::Standard, Self::Micro],
            (false, true) => &[Self::MicroTflite],
            (false, false) => &[],
        }
    }

    pub(crate) fn format(self) -> &'static str {
        match self {
            Self::Standard | Self::Micro => "onnx",
            Self::MicroTflite | Self::MicroWakeWord => "tflite",
            Self::Unknown => "",
        }
    }

    fn matches_tier(self, tier: &str) -> bool {
        matches!(
            (self, tier),
            (Self::Standard, "standard")
                | (
                    Self::Micro | Self::MicroTflite | Self::MicroWakeWord,
                    "micro"
                )
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Selection {
    pub id: String,
    pub wake_word: String,
    pub tier: String,
    pub sha256: String,
    pub runtime: Runtime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(target_os = "espidf", serde(deserialize_with = "threshold_from_json"))]
    pub score_threshold: Option<f32>,
}

#[cfg(target_os = "espidf")]
fn threshold_from_json<'de, D>(deserializer: D) -> std::result::Result<Option<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // The Xtensa backend cannot lower serde_json's direct f32 conversion.
    Ok(Option::<f64>::deserialize(deserializer)?.map(|value| value as f32))
}

impl Selection {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            valid_model_id(&self.id)
                && !self.wake_word.trim().is_empty()
                && self.wake_word.chars().count() <= 80
                && !self.wake_word.chars().any(char::is_control)
                && self.runtime.matches_tier(&self.tier)
                && self
                    .score_threshold
                    .is_none_or(|value| (0.0..=1.0).contains(&value))
                && (!matches!(self.runtime, Runtime::MicroTflite | Runtime::MicroWakeWord)
                    || self
                        .score_threshold
                        .is_some_and(|value| 0.0 < value && value < 1.0))
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "Invalid wake model metadata"
        );
        Ok(())
    }
}

pub(crate) fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

#[derive(Clone, Deserialize)]
#[serde(from = "ModelWire")]
pub(crate) struct Model {
    pub selection: Selection,
    filename: String,
    format: String,
    sample_rate_hz: u32,
    size_bytes: u64,
}

#[derive(Deserialize)]
struct ModelWire {
    id: String,
    wake_word: String,
    tier: String,
    sha256: String,
    runtime: Runtime,
    #[serde(default)]
    #[cfg_attr(target_os = "espidf", serde(deserialize_with = "threshold_from_json"))]
    score_threshold: Option<f32>,
    filename: String,
    format: String,
    sample_rate_hz: u32,
    size_bytes: u64,
}

impl From<ModelWire> for Model {
    fn from(wire: ModelWire) -> Self {
        Self {
            selection: Selection {
                id: wire.id,
                wake_word: wire.wake_word,
                tier: wire.tier,
                sha256: wire.sha256,
                runtime: wire.runtime,
                score_threshold: wire.score_threshold,
            },
            filename: wire.filename,
            format: wire.format,
            sample_rate_hz: wire.sample_rate_hz,
            size_bytes: wire.size_bytes,
        }
    }
}

impl Model {
    #[cfg(target_os = "espidf")]
    pub(crate) fn embedded(selection: Selection, size_bytes: usize) -> Self {
        Self {
            filename: format!("{}.tflite", selection.id),
            format: "tflite".into(),
            sample_rate_hz: 16_000,
            size_bytes: size_bytes as u64,
            selection,
        }
    }

    fn supported(&self, runtimes: &[Runtime]) -> bool {
        self.sample_rate_hz == 16_000
            && runtimes.contains(&self.selection.runtime)
            && self.selection.runtime.format() == self.format
            && self.selection.runtime.matches_tier(&self.selection.tier)
    }

    fn validate(&self) -> Result<()> {
        self.selection.validate()?;
        ensure!(
            self.filename
                .strip_suffix(&format!(".{}", self.format))
                .is_some_and(valid_model_id)
                && (1..=MODEL_LIMIT).contains(&self.size_bytes),
            "Invalid wake model asset"
        );
        Ok(())
    }

    #[cfg(not(target_os = "espidf"))]
    fn copy_verified(&self, input: impl Read, mut output: impl Write) -> Result<()> {
        let mut input = input.take(self.size_bytes + 1);
        let mut buffer = [0; 16 * 1024];
        let mut hash = Sha256Writer::default();
        let mut size = 0;
        loop {
            let bytes = input.read(&mut buffer)?;
            if bytes == 0 {
                break;
            }
            size += bytes as u64;
            ensure!(size <= self.size_bytes, "Wake model size mismatch");
            hash.write_all(&buffer[..bytes])?;
            output.write_all(&buffer[..bytes])?;
        }
        ensure!(size == self.size_bytes, "Wake model size mismatch");
        ensure!(
            hash.finish() == self.selection.sha256,
            "Wake model SHA-256 mismatch"
        );
        Ok(())
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn asset_size(&self) -> u64 {
        self.size_bytes
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn asset_filename(&self) -> &str {
        &self.filename
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn verify_bytes(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() as u64 == self.size_bytes,
            "Wake model size mismatch"
        );
        self.selection.verify_sha256(bytes)
    }
}

#[cfg(target_os = "espidf")]
impl Selection {
    pub(crate) fn verify_sha256(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            crate::checksum::hex(&Sha256::digest(bytes)) == self.sha256,
            "Wake model SHA-256 mismatch"
        );
        Ok(())
    }
}

#[derive(Clone, Deserialize)]
pub(crate) struct Catalog {
    schema_version: u32,
    pub tag: String,
    pub models: Vec<Model>,
}

impl Catalog {
    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn latest(runtimes: &[Runtime]) -> Result<Self> {
        let response = get(&format!("{RELEASES}/latest/download/metadata.json"))
            .context("Download the latest wake model catalog")?;
        let mut metadata = Vec::new();
        response
            .take(CATALOG_LIMIT + 1)
            .read_to_end(&mut metadata)?;
        ensure!(
            metadata.len() as u64 <= CATALOG_LIMIT,
            "Wake model catalog is too large"
        );
        Self::parse(&metadata, runtimes)
    }

    pub(crate) fn parse(metadata: &[u8], runtimes: &[Runtime]) -> Result<Self> {
        let mut catalog: Self = goodwin_json::from_slice(metadata)?;
        ensure!(
            catalog.schema_version == 1,
            "Unsupported wake model catalog version"
        );
        let version = catalog.tag.strip_prefix('v').unwrap_or_default();
        ensure!(
            version.len() <= 32
                && version.split('.').count() == 3
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
            "Invalid wake model release tag"
        );
        catalog.models.retain(|model| model.supported(runtimes));
        let mut ids = HashSet::new();
        for model in &catalog.models {
            model.validate()?;
            ensure!(ids.insert(&model.selection.id), "Duplicate wake model ID");
        }
        Ok(catalog)
    }

    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn download(&self, id: &str, output: impl Write) -> Result<&Selection> {
        let model = self
            .models
            .iter()
            .find(|model| model.selection.id == id)
            .with_context(|| format!("Wake model {id} is missing from release {}", self.tag))?;
        let response = get(&format!(
            "{RELEASES}/download/{}/{}",
            self.tag, model.filename
        ))
        .with_context(|| format!("Download wake model {id}"))?;
        model.copy_verified(response, output)?;
        Ok(&model.selection)
    }
}

#[cfg(not(target_os = "espidf"))]
fn get(url: &str) -> Result<Response> {
    Ok(Client::builder()
        .tls_backend_preconfigured(crate::http_tls::client_config())
        .https_only(true)
        .user_agent("Goodwin")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .build()?
        .get(url)
        .send()
        .and_then(Response::error_for_status)
        .map_err(reqwest::Error::without_url)?)
}

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_only_lists_supported_graphs_and_verifies_the_download() {
        let mut model = json!({
            "id": "egor-micro", "wake_word": "Егор", "tier": "micro",
            "filename": "egor-micro.onnx", "format": "onnx", "sample_rate_hz": 16000,
            "runtime": "livekit-v2-micro-onnx", "goodwin_compatible": true,
            "size_bytes": 3, "sha256": crate::checksum::hex(&Sha256::digest(b"abc"))
        });
        let mut unsupported = model.clone();
        unsupported["format"] = json!("tflite");
        let catalog = |models, with_tflite| {
            Catalog::parse(
                &serde_json::to_vec(&json!({
                    "schema_version": 1, "tag": "v0.1.0", "models": models
                }))
                .unwrap(),
                Runtime::available(true, with_tflite),
            )
        };
        let result = catalog(vec![model.clone(), unsupported], false).unwrap();
        let [entry] = result.models.as_slice() else {
            panic!("Expected one model")
        };
        assert_eq!(entry.selection.wake_word, "Егор");
        let mut output = Vec::new();
        entry.copy_verified(&b"abc"[..], &mut output).unwrap();
        assert_eq!(output, b"abc");
        for bytes in [b"ab".as_slice(), b"abcd", b"abd"] {
            assert!(entry.copy_verified(bytes, std::io::sink()).is_err());
        }
        let mut int8 = model.clone();
        int8["id"] = json!("egor-int8");
        int8["filename"] = json!("egor-int8.tflite");
        int8["format"] = json!("tflite");
        int8["runtime"] = json!("livekit-v2-micro-tflite");
        int8["score_threshold"] = json!(0.390625);
        int8["goodwin_compatible"] = json!(false);
        let mut unknown = int8.clone();
        unknown["runtime"] = json!("unavailable-runtime");
        unknown["goodwin_compatible"] = json!(true);
        let mut unspecified = model.clone();
        unspecified
            .as_object_mut()
            .unwrap()
            .remove("goodwin_compatible");
        let mut microwakeword = int8.clone();
        microwakeword["id"] = json!("goodwin-microwakeword");
        microwakeword["filename"] = json!("goodwin-microwakeword.tflite");
        microwakeword["runtime"] = json!("microwakeword-tflite");
        let models = vec![unspecified, int8.clone(), unknown, microwakeword];
        let esp = Catalog::parse(
            &serde_json::to_vec(&json!({"schema_version":1,"tag":"v0.3.1","models":models}))
                .unwrap(),
            &[Runtime::MicroWakeWord, Runtime::MicroTflite],
        )
        .unwrap();
        assert_eq!(esp.models.len(), 2);
        assert_eq!(esp.models[1].selection.runtime, Runtime::MicroWakeWord);
        assert_eq!(catalog(models.clone(), false).unwrap().models.len(), 1);
        let supported = catalog(models, true).unwrap();
        assert_eq!(supported.models.len(), 2);
        assert_eq!(supported.models[1].selection.runtime, Runtime::MicroTflite);
        int8["filename"] = json!("wrong.onnx");
        assert!(catalog(vec![int8], true).is_err());
        assert!(catalog(vec![model.clone(), model.clone()], true).is_err());
        model["filename"] = json!("../egor-micro.onnx");
        assert!(catalog(vec![model], false).is_err());
    }

    #[test]
    fn selection_requires_an_explicit_runtime() {
        let selection = serde_json::from_value::<Selection>(json!({
            "id": "goodwin-micro", "wake_word": "Гудвин", "tier": "micro",
            "sha256": "0".repeat(64)
        }));
        assert!(selection.is_err());
    }
}

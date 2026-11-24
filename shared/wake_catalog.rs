use std::{
    collections::HashSet,
    io::{Read, Write},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RELEASES: &str = "https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases";
const CATALOG_LIMIT: u64 = 64 * 1024;
const MODEL_LIMIT: u64 = 32 * 1024 * 1024;
pub(crate) fn tflite_marker() -> &'static [u8] {
    include_str!("wake-tflite-capability.txt")
        .trim_end()
        .as_bytes()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum Runtime {
    #[serde(rename = "livekit-v2-standard-onnx")]
    Standard,
    #[serde(rename = "livekit-v2-micro-onnx")]
    Micro,
    #[serde(rename = "livekit-v2-micro-tflite")]
    MicroTflite,
    #[serde(other)]
    Unknown,
}

impl Runtime {
    pub(crate) fn available(with_tflite: bool) -> &'static [Self] {
        if with_tflite {
            &[Self::Standard, Self::Micro, Self::MicroTflite]
        } else {
            &[Self::Standard, Self::Micro]
        }
    }

    pub(crate) fn format(self) -> &'static str {
        match self {
            Self::Standard | Self::Micro => "onnx",
            Self::MicroTflite => "tflite",
            Self::Unknown => "",
        }
    }

    fn matches_tier(self, tier: &str) -> bool {
        matches!(
            (self, tier),
            (Self::Standard, "standard") | (Self::Micro | Self::MicroTflite, "micro")
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Selection {
    pub id: String,
    pub wake_word: String,
    pub tier: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
}

impl Selection {
    pub(crate) fn runtime(&self) -> Runtime {
        self.runtime.unwrap_or(match self.tier.as_str() {
            "standard" => Runtime::Standard,
            "micro" => Runtime::Micro,
            _ => Runtime::Unknown,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            valid_model_id(&self.id)
                && !self.wake_word.trim().is_empty()
                && self.wake_word.chars().count() <= 80
                && !self.wake_word.chars().any(char::is_control)
                && self.runtime().matches_tier(&self.tier)
                && self
                    .score_threshold
                    .is_none_or(|value| (0.0..=1.0).contains(&value))
                && (self.runtime() != Runtime::MicroTflite
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
pub(crate) struct Model {
    #[serde(flatten)]
    pub selection: Selection,
    filename: String,
    format: String,
    sample_rate_hz: u32,
    size_bytes: u64,
}

impl Model {
    fn supported(&self, runtimes: &[Runtime]) -> bool {
        self.sample_rate_hz == 16_000
            && self.selection.runtime.is_some_and(|runtime| {
                runtimes.contains(&runtime)
                    && runtime.format() == self.format
                    && runtime.matches_tier(&self.selection.tier)
            })
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

    fn copy_verified(&self, input: impl Read, mut output: impl Write) -> Result<()> {
        let mut input = input.take(self.size_bytes + 1);
        let mut buffer = [0; 16 * 1024];
        let mut hash = Sha256::new();
        let mut size = 0;
        loop {
            let bytes = input.read(&mut buffer)?;
            if bytes == 0 {
                break;
            }
            size += bytes as u64;
            ensure!(size <= self.size_bytes, "Wake model size mismatch");
            hash.update(&buffer[..bytes]);
            output.write_all(&buffer[..bytes])?;
        }
        ensure!(size == self.size_bytes, "Wake model size mismatch");
        ensure!(
            format!("{:x}", hash.finalize()) == self.selection.sha256,
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

    fn parse(metadata: &[u8], runtimes: &[Runtime]) -> Result<Self> {
        let mut catalog: Self = serde_json::from_slice(metadata)?;
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

fn get(url: &str) -> Result<Response> {
    Ok(Client::builder()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_only_lists_supported_graphs_and_verifies_the_download() {
        let mut model = json!({
            "id": "egor-micro", "wake_word": "Егор", "tier": "micro",
            "filename": "egor-micro.onnx", "format": "onnx", "sample_rate_hz": 16000,
            "runtime": "livekit-v2-micro-onnx", "goodwin_compatible": true,
            "size_bytes": 3, "sha256": format!("{:x}", Sha256::digest(b"abc"))
        });
        let mut unsupported = model.clone();
        unsupported["format"] = json!("tflite");
        let catalog = |models, with_tflite| {
            Catalog::parse(
                &serde_json::to_vec(&json!({
                    "schema_version": 1, "tag": "v0.1.0", "models": models
                }))
                .unwrap(),
                Runtime::available(with_tflite),
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
        let models = vec![unspecified, int8.clone(), unknown];
        assert_eq!(catalog(models.clone(), false).unwrap().models.len(), 1);
        let supported = catalog(models, true).unwrap();
        assert_eq!(supported.models.len(), 2);
        assert_eq!(
            supported.models[1].selection.runtime(),
            Runtime::MicroTflite
        );
        int8["filename"] = json!("wrong.onnx");
        assert!(catalog(vec![int8], true).is_err());
        assert!(catalog(vec![model.clone(), model.clone()], true).is_err());
        model["filename"] = json!("../egor-micro.onnx");
        assert!(catalog(vec![model], false).is_err());
    }

    #[test]
    fn old_manifests_choose_onnx_by_tier() {
        let legacy: Selection = serde_json::from_value(json!({
            "id": "goodwin-micro", "wake_word": "Гудвин", "tier": "micro",
            "sha256": "0".repeat(64)
        }))
        .unwrap();
        legacy.validate().unwrap();
        assert_eq!(legacy.runtime(), Runtime::Micro);
    }
}

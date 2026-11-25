#[cfg(all(target_arch = "wasm32", not(feature = "wasm-guest")))]
use alloc::string::String;
use serde::{Deserialize, Serialize};

use super::ModelSelectionError;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ModelParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl ModelParameters {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub(super) fn validate(&self) -> Result<(), ModelSelectionError> {
        if self.voice.as_ref().is_some_and(|voice| voice.len() > 256)
            || [&self.prompt, &self.instructions]
                .into_iter()
                .flatten()
                .map(String::len)
                .sum::<usize>()
                > 1536
        {
            return Err(ModelSelectionError::ParametersTooLarge);
        }
        if self
            .temperature
            .is_some_and(|value| !(0.0..=2.0).contains(&value))
            || self
                .top_p
                .is_some_and(|value| !(0.0..=1.0).contains(&value))
            || self
                .speed
                .is_some_and(|value| !(0.25..=4.0).contains(&value))
        {
            return Err(ModelSelectionError::InvalidParameters);
        }
        Ok(())
    }
}

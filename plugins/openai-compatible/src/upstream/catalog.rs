use std::{
    cell::Cell,
    collections::HashSet,
    io::{self, BufReader, Read},
};

use goodwin_error::{Result, ensure};
use goodwin_plugin::{json_codec, provider::Model};
use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

const RESPONSE_BYTES: u64 = 8_000_000;
const ENTRY_BYTES: usize = 32 * 1024;
const OUTPUT_BYTES: usize = 450 * 1024;
const OUTPUT_MODELS: usize = 750;

pub(super) fn read(response: impl Read) -> Result<Vec<Model>> {
    let window = Cell::new(None);
    // Place the buffer below the entry limit. Bytes prefetched from HTTP do
    // not count against an entry until the JSON decoder actually reads them.
    let mut input = ModelReader {
        source: BufReader::with_capacity(8 * 1024, response.take(RESPONSE_BYTES + 1)),
        window: &window,
    };
    let models = {
        let mut deserializer = serde_json::Deserializer::from_reader(&mut input);
        let models = deserializer.deserialize_map(Catalog(&window))?;
        deserializer.end()?;
        models
    };
    ensure!(
        input.source.get_ref().limit() > 0,
        "Provider catalog exceeds response limit"
    );
    Ok(models)
}

struct ModelReader<'a, R> {
    source: R,
    window: &'a Cell<Option<usize>>,
}

impl<R: Read> Read for ModelReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(remaining) = self.window.get() else {
            return self.source.read(buffer);
        };
        if remaining == 0 && !buffer.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Provider model entry exceeds size limit",
            ));
        }
        let max = buffer.len().min(remaining);
        let count = self.source.read(&mut buffer[..max])?;
        self.window.set(Some(remaining - count));
        Ok(count)
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum Field {
    Data,
    #[serde(other)]
    Other,
}

struct Catalog<'a>(&'a Cell<Option<usize>>);

impl<'de> Visitor<'de> for Catalog<'_> {
    type Value = Vec<Model>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a provider catalog with a data array")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut catalog = None;
        loop {
            self.0.set(Some(ENTRY_BYTES));
            match map.next_key()? {
                Some(Field::Data) if catalog.is_none() => {
                    catalog = Some(map.next_value_seed(Models(self.0))?);
                }
                Some(Field::Data) => return Err(de::Error::duplicate_field("data")),
                Some(Field::Other) => {
                    map.next_value::<IgnoredAny>()?;
                }
                None => break,
            }
        }
        self.0.set(None);
        catalog.ok_or_else(|| de::Error::missing_field("data"))
    }
}

struct Models<'a>(&'a Cell<Option<usize>>);

impl<'de> DeserializeSeed<'de> for Models<'_> {
    type Value = Vec<Model>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Models<'_> {
    type Value = Vec<Model>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an array of provider models")
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
        let mut models = Vec::new();
        let mut ids = HashSet::new();
        let mut bytes = 0;
        loop {
            self.0.set(Some(ENTRY_BYTES));
            let Some(entry) = seq.next_element::<UpstreamModel>()? else {
                self.0.set(None);
                return Ok(models);
            };
            let Some(model) = entry.supported().filter(|model| !ids.contains(&model.id)) else {
                continue;
            };
            bytes += json_codec::to_vec(&model).map_err(de::Error::custom)?.len() + 1;
            if bytes >= OUTPUT_BYTES {
                break;
            }
            ids.insert(model.id.clone());
            models.push(model);
            if models.len() == OUTPUT_MODELS {
                break;
            }
        }
        // Check the rest of the JSON syntax and the HTTP size limit without
        // constructing any more model records.
        loop {
            self.0.set(Some(ENTRY_BYTES));
            if seq.next_element::<IgnoredAny>()?.is_none() {
                break;
            }
        }
        self.0.set(None);
        Ok(models)
    }
}

#[derive(Deserialize)]
struct UpstreamModel {
    id: String,
    name: Option<String>,
    #[serde(default)]
    output_modalities: Vec<OutputModality>,
}

impl UpstreamModel {
    fn supported(self) -> Option<Model> {
        let id = self.id;
        if id.trim() != id || id.is_empty() || !valid_text(&id, 256) {
            return None;
        }
        let parameters = self
            .output_modalities
            .into_iter()
            .find(|item| item.kind.as_deref() == Some("speech"))
            .and_then(|item| item.supported_parameters);
        if parameters
            .as_ref()
            .and_then(|params| params.response_format.as_ref())
            .and_then(|options| options.values.as_ref())
            .is_some_and(|formats| !formats.iter().any(|format| format == "pcm"))
        {
            return None;
        }
        let voices = parameters
            .and_then(|params| params.voice)
            .and_then(|options| options.values)
            .into_iter()
            .flatten()
            .filter(|voice| !voice.is_empty() && valid_text(voice, 100))
            .take(100)
            .collect();
        Some(Model {
            id,
            name: self.name.filter(|name| valid_text(name, 256)),
            voices,
        })
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.chars().any(char::is_control)
}

#[derive(Deserialize)]
struct OutputModality {
    #[serde(rename = "type")]
    kind: Option<String>,
    supported_parameters: Option<SpeechOptions>,
}

#[derive(Deserialize)]
struct SpeechOptions {
    response_format: Option<Options>,
    voice: Option<Options>,
}

#[derive(Deserialize)]
struct Options {
    values: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_unique_pcm_models_and_bounded_voice_labels() {
        let catalog = br#"{"data":[
            {"id":"speech","output_modalities":[{"type":"speech","supported_parameters":{"response_format":{"values":["mp3"]}}}]},
            {"id":"speech","name":"Speech","output_modalities":[{"type":"speech","supported_parameters":{
                "response_format":{"values":["pcm"]},"voice":{"values":["alloy","","bad\u0000"]}}}]},
            {"id":"speech"}, {"id":" bad "}, {"id":"chat","name":"bad\u0000"}
        ],"object":"list"}"#;
        let models = read(catalog.as_slice()).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "speech");
        assert_eq!(models[0].voices, ["alloy"]);
        assert_eq!(models[1].id, "chat");
        assert!(models[1].name.is_none());
    }

    #[test]
    fn stops_constructing_records_at_the_output_limit_and_validates_the_tail() {
        let prefix = (0..=OUTPUT_MODELS)
            .map(|id| format!(r#"{{"id":"model-{id}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let valid = format!(
            r#"{{"data":[{prefix},{{"unused":"{}"}}]}}"#,
            "x".repeat(ENTRY_BYTES / 2)
        );
        assert_eq!(read(valid.as_bytes()).unwrap().len(), OUTPUT_MODELS);
        let invalid = format!(r#"{{"data":[{prefix},{{"bad":}}]}}"#);
        assert!(read(invalid.as_bytes()).is_err());
    }

    #[test]
    fn rejects_oversized_entries_and_responses() {
        let oversized = format!(
            r#"{{"data":[{{"id":"valid","name":"{}"}}]}}"#,
            "x".repeat(ENTRY_BYTES)
        );
        let error = read(oversized.as_bytes()).err().unwrap();
        assert!(error.to_string().contains("entry exceeds size limit"));
        let response = b"{\"data\":[]}"
            .as_slice()
            .chain(io::repeat(b' ').take(RESPONSE_BYTES));
        let error = read(response).err().unwrap();
        assert!(error.to_string().contains("catalog exceeds response limit"));
    }
}

use std::{collections::BTreeMap, path::Path};

use anyhow::Result;
use goodwin_plugin::settings::SettingsData;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Failure, package, settings::valid_base_url};

const FILE: &str = ".goodwin-provider-connections.json";
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";
const LIMIT: usize = 16;

pub(crate) fn valid_provider_id(id: &str) -> bool {
    id.split_once(':').map_or_else(
        || package::name(id),
        |(plugin, connection)| package::name(plugin) && package::name(connection),
    )
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Protocol {
    OpenAi,
    OpenRouter,
}

impl Protocol {
    pub(super) fn speech_streaming(self) -> bool {
        matches!(self, Self::OpenAi)
    }
    pub(super) fn requires_key(self) -> bool {
        matches!(self, Self::OpenRouter)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Connection {
    name: String,
    pub(super) protocol: Protocol,
    pub(super) base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) key: String,
}

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Store {
    revision: u64,
    entries: BTreeMap<String, Connection>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    pub(super) id: String,
    pub(super) provider_id: String,
    name: String,
    protocol: Protocol,
    base_url: String,
    has_key: bool,
    pub(super) speech_streaming: bool,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Update {
    id: Option<String>,
    name: String,
    protocol: Protocol,
    base_url: String,
    key: Option<String>,
}

impl Store {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn read(directory: &Path) -> Result<Self> {
        goodwin_plugin::read_json(&directory.join(FILE))
    }

    #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
    pub(super) fn connection(&self, id: &str) -> Option<&Connection> {
        self.entries.get(id)
    }

    pub(super) fn load_or_migrate(
        plugin: &str,
        directory: &Path,
        openrouter: &Path,
    ) -> Result<Self> {
        if directory.join(FILE).try_exists()? {
            return Self::read(directory);
        }
        let mut store = Self::default();
        if plugin == "openai-compatible" {
            let legacy = SettingsData::read(directory)?;
            if let Some(base_url) = legacy
                .values
                .get("base_url")
                .and_then(Value::as_str)
                .filter(|url| valid_base_url(url))
            {
                store.entries.insert(
                    "default".into(),
                    Connection {
                        name: "OpenAI-compatible".into(),
                        protocol: Protocol::OpenAi,
                        base_url: base_url.into(),
                        key: legacy.secrets.get("key").cloned().unwrap_or_default(),
                    },
                );
            }
            let legacy = SettingsData::read(openrouter)?;
            store.entries.insert(
                "openrouter".into(),
                Connection {
                    name: "OpenRouter".into(),
                    protocol: Protocol::OpenRouter,
                    base_url: OPENROUTER_URL.into(),
                    key: legacy.secrets.get("key").cloned().unwrap_or_default(),
                },
            );
        }
        store.save(directory)?;
        Ok(store)
    }

    pub(super) fn save(&self, directory: &Path) -> Result<()> {
        goodwin_plugin::save_json(&directory.join(FILE), self)
    }

    pub(super) fn view(&self, plugin: &str) -> Vec<View> {
        self.entries
            .iter()
            .map(|(id, connection)| View {
                id: id.clone(),
                provider_id: match (plugin, id.as_str()) {
                    ("openai-compatible", "default") => plugin.into(),
                    ("openai-compatible", "openrouter") => "openrouter".into(),
                    _ => format!("{plugin}:{id}"),
                },
                name: connection.name.clone(),
                protocol: connection.protocol,
                base_url: connection.base_url.clone(),
                has_key: !connection.key.is_empty(),
                speech_streaming: connection.protocol.speech_streaming(),
                revision: self.revision,
            })
            .collect()
    }

    pub(super) fn update(
        &mut self,
        update: Update,
        new_id: impl FnOnce() -> Result<String>,
    ) -> Result<String, Failure> {
        let name = update.name.trim();
        let base_url = match update.protocol {
            Protocol::OpenAi => update.base_url.trim(),
            Protocol::OpenRouter => OPENROUTER_URL,
        };
        if name.is_empty()
            || name.len() > 160
            || name.chars().any(char::is_control)
            || base_url.len() > 2048
            || !valid_base_url(base_url)
            || update.key.as_ref().is_some_and(|key| {
                key.len() > 512 || !key.bytes().all(|byte| byte.is_ascii_graphic())
            })
        {
            return Err(Failure::InvalidSettings);
        }
        let (id, previous_key) = if let Some(id) = update.id {
            let previous = self.entries.get(&id).ok_or(Failure::NotFound)?;
            if (previous.base_url != base_url
                || previous.protocol.requires_key() != update.protocol.requires_key())
                && !previous.key.is_empty()
                && update.key.is_none()
            {
                return Err(Failure::InvalidSettings);
            }
            (id, previous.key.clone())
        } else {
            if self.entries.len() >= LIMIT {
                return Err(Failure::InvalidSettings);
            }
            let id = new_id().map_err(super::store::storage)?;
            if self.entries.contains_key(&id) {
                return Err(Failure::Storage);
            }
            (id, String::new())
        };
        self.entries.insert(
            id.clone(),
            Connection {
                name: name.into(),
                protocol: update.protocol,
                base_url: base_url.into(),
                key: update.key.unwrap_or(previous_key),
            },
        );
        self.revision += 1;
        Ok(id)
    }

    pub(super) fn remove(&mut self, id: &str) -> Result<(), Failure> {
        self.entries.remove(id).ok_or(Failure::NotFound)?;
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changing_a_connection_url_requires_an_explicit_key_change() {
        let mut store = Store::default();
        let update = |json| serde_json::from_value(json).unwrap();
        let initial = update(
            json!({"name":"Local", "protocol":"openai", "baseUrl":"http://first.test/v1", "key":"secret"}),
        );
        assert!(store.update(initial, || Ok("fixture".into())).is_ok());
        let base = json!({"id":"fixture", "name":"Remote", "protocol":"openai", "baseUrl":"https://second.test/v1"});
        assert!(matches!(
            store.update(update(base.clone()), || unreachable!()),
            Err(Failure::InvalidSettings)
        ));
        assert_eq!(store.entries["fixture"].base_url, "http://first.test/v1");
        let mut cleared = base;
        cleared["key"] = json!("");
        assert!(store.update(update(cleared), || unreachable!()).is_ok());
        assert!(store.entries["fixture"].key.is_empty());
        assert_eq!(store.entries["fixture"].base_url, "https://second.test/v1");
    }
}

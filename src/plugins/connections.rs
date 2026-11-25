use std::{collections::BTreeMap, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "espidf"))]
use sha2::{Digest, Sha256};

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
pub(crate) enum Protocol {
    OpenAi,
    OpenRouter,
}

impl Protocol {
    pub(crate) fn speech_streaming(self) -> bool {
        matches!(self, Self::OpenAi)
    }
    pub(crate) fn requires_key(self) -> bool {
        matches!(self, Self::OpenRouter)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Connection {
    name: String,
    pub(crate) protocol: Protocol,
    pub(crate) base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) key: String,
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct Store {
    revision: u64,
    entries: BTreeMap<String, Connection>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    pub(crate) id: String,
    pub(crate) provider_id: String,
    name: String,
    protocol: Protocol,
    base_url: String,
    has_key: bool,
    pub(crate) speech_streaming: bool,
    revision: u64,
    #[serde(skip)]
    #[cfg(not(target_os = "espidf"))]
    pub(crate) request_configuration: [u8; 32],
    #[serde(skip)]
    #[cfg(not(target_os = "espidf"))]
    pub(crate) request_revision: u64,
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
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn read(directory: &Path) -> Result<Self> {
        goodwin_plugin::read_json(&directory.join(FILE))
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn read(directory: &Path) -> Result<Self> {
        crate::platform::espidf::plugins::services::read_json(&directory.join(FILE))
    }

    #[cfg(all(
        feature = "wasm-plugins",
        any(target_os = "linux", target_os = "espidf")
    ))]
    pub(crate) fn connection(&self, id: &str) -> Option<&Connection> {
        self.entries.get(id)
    }

    pub(crate) fn initialize(plugin: &str, directory: &Path) -> Result<Self> {
        if directory.join(FILE).try_exists()? {
            return Self::read(directory);
        }

        let mut store = Self::default();
        if plugin == "openai-compatible" {
            store.entries.insert(
                "openrouter".into(),
                Connection {
                    name: "OpenRouter".into(),
                    protocol: Protocol::OpenRouter,
                    base_url: OPENROUTER_URL.into(),
                    key: String::new(),
                },
            );
        }
        store.save(directory)?;
        Ok(store)
    }

    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn save(&self, directory: &Path) -> Result<()> {
        goodwin_plugin::save_json(&directory.join(FILE), self)
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn save(&self, directory: &Path) -> Result<()> {
        crate::platform::espidf::plugins::services::save_json(
            &directory.join(FILE),
            self,
            64 * 1024,
        )
    }

    pub(crate) fn view(&self, plugin: &str) -> Vec<View> {
        self.entries
            .iter()
            .map(|(id, connection)| View {
                id: id.clone(),
                provider_id: format!("{plugin}:{id}"),
                name: connection.name.clone(),
                protocol: connection.protocol,
                base_url: connection.base_url.clone(),
                has_key: !connection.key.is_empty(),
                speech_streaming: connection.protocol.speech_streaming(),
                revision: self.revision,
                #[cfg(not(target_os = "espidf"))]
                request_configuration: Sha256::digest(
                    serde_json::to_vec(&(
                        &connection.protocol,
                        &connection.base_url,
                        &connection.key,
                    ))
                    .expect("connection configuration is serializable"),
                )
                .into(),
                #[cfg(not(target_os = "espidf"))]
                request_revision: self.revision,
            })
            .collect()
    }

    pub(crate) fn update(
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

    pub(crate) fn remove(&mut self, id: &str) -> Result<(), Failure> {
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
    fn openrouter_is_created_only_when_the_connection_store_is_new() {
        let directory = tempfile::tempdir().unwrap();
        let plugin = "openai-compatible";
        let mut store = Store::initialize(plugin, directory.path()).unwrap();
        let views = store.view(plugin);
        let [connection] = views.as_slice() else {
            panic!("Expected the initial OpenRouter connection");
        };
        assert_eq!(connection.provider_id, "openai-compatible:openrouter");
        assert!(!connection.has_key);

        assert!(store.remove(&connection.id).is_ok());
        store.save(directory.path()).unwrap();
        let reopened = Store::initialize(plugin, directory.path()).unwrap();
        assert!(reopened.view(plugin).is_empty());
    }

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

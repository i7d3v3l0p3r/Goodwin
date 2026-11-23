use std::{collections::BTreeMap, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::{data_dir, read_json, save_json};

pub const SETTINGS_FILE: &str = ".goodwin-settings.json";

#[derive(Default, Deserialize, Serialize)]
pub struct SettingsData {
    pub values: Map<String, Value>,
    pub secrets: BTreeMap<String, String>,
}

impl SettingsData {
    pub fn read(directory: &Path) -> Result<Self> {
        read_json(&directory.join(SETTINGS_FILE))
    }

    pub fn save(&self, directory: &Path) -> Result<()> {
        save_json(&directory.join(SETTINGS_FILE), self)
    }
}

/// Load the current settings. Use Serde defaults for fields the user can leave unset.
pub fn read_settings<T: DeserializeOwned>() -> Result<T> {
    let SettingsData {
        mut values,
        secrets,
    } = SettingsData::read(&data_dir()?)?;
    values.extend(
        secrets
            .into_iter()
            .map(|(id, secret)| (id, Value::String(secret))),
    );
    Ok(serde_json::from_value(Value::Object(values))?)
}

use std::collections::HashSet;

use anyhow::{Result, ensure};
use goodwin_plugin::settings::SettingsData;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

use super::{Failure, package::Label};

#[cfg(test)]
mod tests;

// Allow 24 maximum-length values even if every byte uses a JSON Unicode escape.
pub(crate) const BODY_LIMIT: usize = 320 * 1024;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Text,
    Url,
    Secret,
    Boolean,
    Integer,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Field {
    id: String,
    label: Label,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Label>,
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(default)]
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
}

impl Field {
    fn length_limit(&self) -> usize {
        match self.kind {
            Kind::Text => 1024,
            Kind::Url => 2048,
            Kind::Secret => 512,
            Kind::Boolean | Kind::Integer => 0,
        }
    }

    fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            Kind::Boolean => value.is_boolean(),
            Kind::Integer => value
                .as_i64()
                .is_some_and(|n| n.unsigned_abs() < (1_u64 << 53)),
            kind => value.as_str().is_some_and(|text| {
                !text.is_empty()
                    && text.len() <= self.max_length.unwrap_or_else(|| self.length_limit())
                    && !text.chars().any(char::is_control)
                    && match kind {
                        Kind::Url => valid_base_url(text),
                        _ => true,
                    }
            }),
        }
    }
}

pub(super) fn valid_base_url(text: &str) -> bool {
    Url::parse(text).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

pub(super) fn wasm_http_fields(fields: &[Field], base: Option<&str>, key: &str) -> bool {
    base.is_none_or(|id| {
        fields
            .iter()
            .any(|field| field.id == id && field.kind == Kind::Url)
    }) && fields
        .iter()
        .any(|field| field.id == key && field.kind == Kind::Secret)
}

pub(super) fn validate(fields: &[Field]) -> Result<()> {
    ensure!(fields.len() <= 24, "Too many plugin settings");
    let mut ids = HashSet::new();
    for field in fields {
        let mut bytes = field.id.bytes();
        ensure!(
            field.id.len() <= 40
                && bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && ids.insert(&field.id),
            "Invalid or repeated plugin setting ID"
        );
        ensure!(
            field.label.valid() && field.description.as_ref().is_none_or(Label::valid),
            "Invalid plugin setting label or description"
        );
        ensure!(
            field
                .max_length
                .is_none_or(|length| (1..=field.length_limit()).contains(&length)),
            "Invalid plugin setting length limit"
        );
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    fields: Vec<Field>,
    values: Map<String, Value>,
    configured_secrets: Vec<String>,
}

impl View {
    pub(super) fn new(fields: Vec<Field>, data: SettingsData) -> Self {
        let values = fields
            .iter()
            .filter_map(|field| {
                let value = data.values.get(&field.id)?;
                (field.kind != Kind::Secret && field.accepts(value))
                    .then(|| (field.id.clone(), value.clone()))
            })
            .collect();
        let configured_secrets = fields
            .iter()
            .filter(|field| field.kind == Kind::Secret && data.secrets.contains_key(&field.id))
            .map(|field| field.id.clone())
            .collect();
        Self {
            fields,
            values,
            configured_secrets,
        }
    }
}

pub(super) fn apply(
    fields: &[Field],
    mut data: SettingsData,
    patch: Map<String, Value>,
) -> Result<SettingsData, Failure> {
    for (id, value) in patch {
        let field = fields
            .iter()
            .find(|field| field.id == id)
            .ok_or(Failure::InvalidSettings)?;
        let value = match value {
            value if field.kind == Kind::Secret => value,
            Value::String(text) if text.trim().is_empty() => Value::Null,
            Value::String(text) => Value::String(text.trim().into()),
            value => value,
        };
        data.values.remove(&id);
        data.secrets.remove(&id);
        if value.is_null() {
            continue;
        }
        if !field.accepts(&value) {
            return Err(Failure::InvalidSettings);
        }
        if field.kind == Kind::Secret {
            if let Value::String(secret) = value {
                data.secrets.insert(id, secret);
            }
        } else {
            data.values.insert(id, value);
        }
    }
    for field in fields.iter().filter(|field| field.required) {
        let valid = if field.kind == Kind::Secret {
            data.secrets
                .get(&field.id)
                .is_some_and(|secret| field.accepts(&Value::String(secret.clone())))
        } else {
            data.values
                .get(&field.id)
                .is_some_and(|value| field.accepts(value))
        };
        if !valid {
            return Err(Failure::InvalidSettings);
        }
    }
    Ok(data)
}

use std::collections::{BTreeSet, HashSet};

use goodwin_error::{Context, Result, ensure};
use goodwin_plugin::provider::{Capability, ToolParameterKind, ToolSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::package::{self, API_VERSION, Label, Manifest};

#[derive(Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Option<Value>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub method: String,
    pub path: String,
    // Existing packages can still send this field. The panel authorizes all plugin routes.
    #[serde(default, rename = "password_required", skip_serializing)]
    _password_required: bool,
}

impl Route {
    pub fn accepts(&self, method: &str, path: &str) -> bool {
        self.method == method
            && self.path.split('/').count() == path.split('/').count()
            && self
                .path
                .split('/')
                .zip(path.split('/'))
                .all(|(template, part)| {
                    template == part || template.starts_with(':') && !part.is_empty()
                })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub path: String,
    pub title: Label,
    #[serde(default)]
    pub navigation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: ToolSchema,
}

impl Tool {
    fn valid(&self) -> bool {
        let properties = &self.parameters.properties;
        package::name(&self.name)
            && !self.description.trim().is_empty()
            && self.description.len() <= 1000
            && !self.parameters.additional_properties
            && properties.len() <= 16
            && self
                .parameters
                .required
                .iter()
                .all(|key| properties.contains_key(key))
    }

    pub fn accepts(&self, args: &Value) -> bool {
        let Some(args) = args.as_object() else {
            return false;
        };
        self.parameters
            .required
            .iter()
            .all(|key| args.contains_key(key))
            && args.iter().all(|(key, value)| {
                match self
                    .parameters
                    .properties
                    .get(key)
                    .map(|property| property.kind)
                {
                    Some(ToolParameterKind::String) => {
                        value.as_str().is_some_and(|value| value.len() <= 1000)
                    }
                    Some(ToolParameterKind::Integer) => value.is_i64() || value.is_u64(),
                    Some(ToolParameterKind::Number) => value.is_number(),
                    Some(ToolParameterKind::Boolean) => value.is_boolean(),
                    None => false,
                }
            })
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    api_version: u32,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub voice_context_tool: Option<String>,
    #[serde(default)]
    pub providers: BTreeSet<Capability>,
    #[serde(default)]
    pub model_catalog: bool,
    #[serde(default)]
    pub(crate) chat_streaming: bool,
    #[serde(default)]
    pub(crate) speech_streaming: bool,
    #[serde(default)]
    pub(crate) wake_events: bool,
    #[serde(default)]
    pub(crate) wake_subscription: u64,
    #[serde(default)]
    pub(crate) timer_alarms: bool,
}

impl Registration {
    #[cfg(any(
        target_os = "espidf",
        all(feature = "wasm-plugins", target_os = "linux")
    ))]
    pub(crate) fn validate_wasm(&self, manifest: &Manifest) -> Result<()> {
        self.validate(manifest)?;
        ensure!(
            (manifest.wasm_http.is_some()
                || (self.providers.is_empty()
                    && !self.model_catalog
                    && !self.chat_streaming
                    && !self.speech_streaming))
                && (manifest.wasm_storage || !self.wake_events),
            "WASM plugin registration requires host services"
        );
        Ok(())
    }

    pub(crate) fn validate(&self, manifest: &Manifest) -> Result<()> {
        ensure!(
            self.api_version == API_VERSION,
            "Plugin RPC version mismatch"
        );
        ensure!(
            self.pages.len() <= 16 && self.routes.len() <= 32 && self.tools.len() <= 16,
            "Too many plugin registrations"
        );
        ensure!(
            self.pages.is_empty() || manifest.web.is_some(),
            "Pages require a web entry module"
        );
        let mut paths = HashSet::new();
        let mut ids = HashSet::new();
        for page in &self.pages {
            ensure!(
                manifest.settings.is_empty() || page.path != "settings",
                "The settings page path is reserved for plugins with declarative settings"
            );
            ensure!(
                package::name(&page.id)
                    && page.title.valid()
                    && (page.path.is_empty() || page.path.split('/').all(package::name))
                    && page.path.len() <= 128,
                "Invalid plugin page registration"
            );
            ensure!(
                paths.insert(&page.path) && ids.insert(&page.id),
                "Duplicate plugin page"
            );
        }
        let mut routes = HashSet::new();
        for route in &self.routes {
            let path = route
                .path
                .strip_prefix('/')
                .context("Plugin API routes are absolute within the plugin")?;
            ensure!(
                matches!(
                    route.method.as_str(),
                    "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
                ) && path.len() <= 128
                    && path
                        .split('/')
                        .all(|part| package::name(part.strip_prefix(':').unwrap_or(part))),
                "Invalid plugin API registration"
            );
            let shape = path
                .split('/')
                .map(|part| if part.starts_with(':') { ":" } else { part })
                .collect::<Vec<_>>()
                .join("/");
            ensure!(
                routes.insert((&route.method, shape)),
                "Duplicate plugin API route"
            );
        }
        let mut tools = HashSet::new();
        for tool in &self.tools {
            ensure!(
                tool.valid() && tools.insert(&tool.name),
                "Invalid or repeated assistant tool"
            );
        }
        if let Some(name) = &self.voice_context_tool {
            ensure!(
                self.tools
                    .iter()
                    .any(|tool| tool.name == *name && tool.accepts(&json!({}))),
                "Voice context must reference a tool that accepts empty arguments"
            );
        }
        Ok(())
    }
}

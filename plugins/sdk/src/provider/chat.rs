#[cfg(all(target_arch = "wasm32", not(feature = "wasm-guest")))]
use alloc::{collections::BTreeMap, string::String, vec::Vec};
#[cfg(not(all(target_arch = "wasm32", not(feature = "wasm-guest"))))]
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ChatRequest, ToolChoice};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    #[default]
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ChatMessage {
    #[serde(default)]
    pub role: ChatRole,
    pub content: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
}

impl ChatMessage {
    pub fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            ..Self::default()
        }
    }

    pub fn tool_result(id: String, content: String) -> Self {
        Self {
            tool_call_id: Some(id),
            ..Self::text(ChatRole::Tool, content)
        }
    }
}

fn optional_list<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ReasoningDetail {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub text: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub signature: Option<Option<String>>,
}

fn nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    #[default]
    Function,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: ToolKind,
    pub function: FunctionCall,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    kind: ToolKind,
    function: ToolFunction,
}

impl ToolDefinition {
    pub fn function(name: String, description: String, parameters: ToolSchema) -> Self {
        Self {
            kind: ToolKind::Function,
            function: ToolFunction {
                name,
                description,
                parameters,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ToolFunction {
    name: String,
    description: String,
    parameters: ToolSchema,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchema {
    #[serde(rename = "type")]
    pub kind: ToolSchemaKind,
    #[serde(deserialize_with = "crate::json_codec::string_map::deserialize")]
    pub properties: BTreeMap<String, ToolProperty>,
    pub required: Vec<String>,
    pub additional_properties: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSchemaKind {
    Object,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolProperty {
    #[serde(rename = "type")]
    pub kind: ToolParameterKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolParameterKind {
    String,
    Integer,
    Number,
    Boolean,
}

#[derive(Deserialize)]
pub struct ChatCompletion {
    pub choices: Vec<super::Completion>,
}

#[derive(Serialize)]
pub struct ChatPayload<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "<[ToolDefinition]>::is_empty")]
    pub tools: &'a [ToolDefinition],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    pub max_completion_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderOptions>,
}

#[derive(Serialize)]
struct ProviderOptions {
    require_parameters: bool,
}

impl<'a> ChatPayload<'a> {
    pub fn new(model: &'a str, request: &'a ChatRequest, stream: bool, openrouter: bool) -> Self {
        Self {
            model,
            messages: &request.messages,
            tools: &request.tools,
            tool_choice: request.tool_choice,
            max_completion_tokens: request.max_completion_tokens,
            temperature: request.selection.parameters.temperature,
            top_p: request.selection.parameters.top_p,
            stream: stream.then_some(true),
            provider: (openrouter && !request.tools.is_empty()).then_some(ProviderOptions {
                require_parameters: true,
            }),
        }
    }
}

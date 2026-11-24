use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use goodwin_plugin::provider::{ChatRequest, ToolChoice};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub(crate) struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

#[derive(Deserialize)]
pub(crate) struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub(crate) struct CompletionMessage(pub(super) Value);

impl CompletionMessage {
    pub(crate) fn text(&self) -> Result<String> {
        let content = self.0["content"].as_str().unwrap_or_default().trim();
        ensure!(!content.is_empty(), "Empty provider answer");
        Ok(content.to_owned())
    }

    pub(crate) fn tool_calls(&self) -> Result<Vec<ToolCall>> {
        match self.0.get("tool_calls") {
            Some(calls) if !calls.is_null() => {
                serde_json::from_value(calls.clone()).context("Invalid provider tool calls")
            }
            _ => Ok(Vec::new()),
        }
    }
}

pub(crate) struct ToolConversation {
    messages: Vec<Value>,
    definitions: Vec<Value>,
}

impl ToolConversation {
    pub(crate) fn new<'a>(
        instructions: &str,
        text: &str,
        tools: impl Iterator<Item = (&'a str, &'a str, &'a Value)>,
        context: BTreeMap<String, Value>,
    ) -> Self {
        let definitions = tools.map(|(name, description, parameters)| json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters }
        })).collect();
        let mut messages = vec![json!({"role": "system", "content": instructions})];
        if !context.is_empty() {
            messages.push(
                json!({"role": "user", "content": json!({"pluginContext": context}).to_string()}),
            );
        }
        messages.push(json!({"role": "user", "content": text}));
        Self {
            messages,
            definitions,
        }
    }

    pub(crate) fn push_completion(&mut self, message: CompletionMessage) {
        self.messages.push(message.0);
    }

    pub(crate) fn push_tool_result(&mut self, id: &str, result: &Value) {
        self.messages.push(json!({
            "role": "tool", "tool_call_id": id, "content": result.to_string()
        }));
    }

    pub(crate) fn request(&self, can_call: bool) -> ChatRequest {
        ChatRequest {
            messages: self.messages.clone(),
            tools: self.definitions.clone(),
            tool_choice: Some(if can_call {
                ToolChoice::Auto
            } else {
                ToolChoice::None
            }),
            max_completion_tokens: 256,
            selection: Default::default(),
        }
    }
}

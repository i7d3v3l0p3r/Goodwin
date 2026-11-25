use std::{collections::BTreeMap, time::Duration};

use goodwin_plugin::provider::ToolSchema;
use serde_json::{Value, json};
use tokio::{task::JoinSet, time::timeout};

use super::process::{Process, Tool};

pub struct VoiceTools {
    pub(super) tools: BTreeMap<String, (Process, Tool)>,
    pub(super) context_tools: BTreeMap<String, String>,
    pub(super) turn: u64,
}

impl VoiceTools {
    #[cfg(test)]
    pub fn mock(handler: impl Fn(Value) -> Value + Send + 'static) -> Self {
        let tool = Tool {
            name: "example".into(),
            description: "Run the test tool".into(),
            parameters: serde_json::from_value(json!({
                "type": "object", "properties": {"seconds": {"type": "integer"}, "name": {"type": "string"}},
                "required": [], "additionalProperties": false,
            })).unwrap(),
        };
        let tools = BTreeMap::from([("probe__example".into(), (Process::mock(handler), tool))]);
        Self {
            tools,
            context_tools: BTreeMap::new(),
            turn: 3,
        }
    }

    #[cfg(test)]
    pub fn with_mock_context(mut self, handler: impl Fn(Value) -> Value + Send + 'static) -> Self {
        let tool = Tool {
            name: "list".into(),
            description: "List test timers".into(),
            parameters: serde_json::from_value(json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false})).unwrap(),
        };
        let name = "probe__list".to_owned();
        self.tools
            .insert(name.clone(), (Process::mock(handler), tool));
        self.context_tools.insert("probe".into(), name);
        self
    }

    pub(crate) fn definitions(&self) -> impl ExactSizeIterator<Item = (&str, &str, &ToolSchema)> {
        self.tools
            .iter()
            .map(|(name, (_, tool))| (name.as_str(), tool.description.as_str(), &tool.parameters))
    }

    pub(crate) async fn context(&self) -> BTreeMap<String, Value> {
        const CONTEXT_TIMEOUT: Duration = Duration::from_millis(300);
        const CONTEXT_LIMIT: usize = 32 * 1024;
        let mut pending = JoinSet::new();
        for (id, name) in &self.context_tools {
            let Some((process, tool)) = self.tools.get(name) else {
                continue;
            };
            let (id, process) = (id.clone(), process.clone());
            let args = json!({"name": tool.name, "arguments": {}, "voiceTurn": self.turn});
            pending.spawn(async move {
                let data = timeout(CONTEXT_TIMEOUT, process.call("tool", args))
                    .await
                    .ok()?
                    .ok()?;
                (data.is_object() && data.get("error").is_none()).then_some((id, data))
            });
        }
        let mut snapshots = BTreeMap::new();
        while let Some(result) = pending.join_next().await {
            if let Ok(Some((id, data))) = result {
                snapshots.insert(id, data);
            }
        }
        let mut bytes = 0;
        snapshots.retain(|id, data| {
            let size = id.len() + data.to_string().len();
            if bytes + size > CONTEXT_LIMIT {
                return false;
            }
            bytes += size;
            true
        });
        snapshots
    }

    pub async fn call(&self, name: &str, arguments: &str) -> Value {
        let Some((process, tool)) = self.tools.get(name) else {
            return json!({"error": "unknown_tool"});
        };
        let Ok(args) = serde_json::from_str(arguments) else {
            return json!({"error": "invalid_arguments"});
        };
        if !tool.accepts(&args) {
            return json!({"error": "invalid_arguments"});
        }
        process
            .call(
                "tool",
                json!({"name": tool.name, "arguments": args, "voiceTurn": self.turn}),
            )
            .await
            .unwrap_or_else(|_| json!({"error": "tool_unavailable"}))
    }
}

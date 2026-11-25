use anyhow::Result;

use crate::{
    builtin_assistant::{Provider, ProviderSettings},
    panel_core::Message,
};

use super::http_client::HttpClient;

pub(super) struct Cascade<'a> {
    settings: &'a ProviderSettings,
    builtin: Provider<'a, HttpClient>,
    prompt: &'a str,
    #[cfg(feature = "wasm-plugins")]
    pub(super) plugins: Option<&'a super::plugins::Plugins>,
    #[cfg(feature = "wasm-plugins")]
    pub(super) assistant: Option<&'a crate::assistant::AssistantSettings>,
}

impl<'a> Cascade<'a> {
    pub(super) fn new(settings: &'a ProviderSettings, prompt: &'a str) -> Self {
        Self {
            settings,
            builtin: Provider::new(settings, HttpClient),
            prompt,
            #[cfg(feature = "wasm-plugins")]
            plugins: None,
            #[cfg(feature = "wasm-plugins")]
            assistant: None,
        }
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn with_plugins(
        mut self,
        plugins: Option<&'a super::plugins::Plugins>,
        assistant: Option<&'a crate::assistant::AssistantSettings>,
    ) -> Self {
        self.plugins = plugins;
        self.assistant = assistant;
        self
    }

    pub(super) fn https(&self) -> bool {
        #[cfg(all(feature = "wasm-plugins", not(feature = "esp-emulator")))]
        if self.assistant.is_some() {
            return true;
        }
        self.settings.https()
    }

    #[cfg(feature = "wasm-plugins")]
    fn plugin(
        &self,
        capability: goodwin_plugin::provider::Capability,
    ) -> Result<Option<super::plugins::PluginProvider<'_>>> {
        let Some(stage) = self
            .assistant
            .map(|settings| settings.pipeline.get(capability))
            .filter(|stage| stage.provider != crate::builtin_assistant::BUILTIN_PROVIDER)
        else {
            return Ok(None);
        };
        let plugin = self
            .plugins
            .ok_or_else(|| anyhow::anyhow!("Plugin runtime is unavailable"))?;
        Ok(Some(super::plugins::PluginProvider {
            plugin,
            settings: stage,
        }))
    }

    pub(super) fn transcribe(&self, wav: &[u8]) -> Result<String> {
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Transcription)? {
            return provider.transcribe(wav);
        }
        self.builtin.transcribe(wav)
    }

    pub(super) fn chat(
        &self,
        question: &str,
        history: &[Message],
        consume: impl FnMut(&str),
    ) -> Result<String> {
        let instructions = if self.prompt.is_empty() {
            crate::panel_core::CHAT_INSTRUCTIONS
        } else {
            self.prompt
        };
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            let request = chat_request(instructions, question, history);
            if provider.streaming(goodwin_plugin::provider::Capability::Chat) {
                return provider.stream_chat(request, consume);
            }
            return provider.complete(request)?.text();
        }
        let _ = consume;
        self.builtin.chat(question, history, instructions)
    }

    pub(super) fn reply(&self, text: &str) -> Result<String> {
        let instructions = self.voice_instructions();
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            return provider
                .complete(chat_request(&instructions, text, &[]))?
                .text();
        }
        self.builtin.reply(text, &instructions)
    }

    pub(super) fn voice_instructions(&self) -> std::borrow::Cow<'_, str> {
        crate::voice_core::instructions(self.prompt)
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn complete_tools(
        &self,
        request: goodwin_plugin::provider::ChatRequest,
    ) -> Result<crate::voice_core::tools::CompletionMessage> {
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            return provider.complete(request);
        }
        self.builtin.complete_tools(request)
    }

    pub(super) fn synthesize(&self, answer: &str) -> Result<Vec<u8>> {
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Speech)? {
            return provider.synthesize(answer);
        }
        self.builtin.synthesize(answer)
    }

    pub(super) fn streaming(&self) -> bool {
        #[cfg(feature = "wasm-plugins")]
        if let Some(assistant) = self.assistant {
            return assistant.pipeline.speech_streaming
                && self
                    .plugin(goodwin_plugin::provider::Capability::Speech)
                    .is_ok_and(|provider| {
                        provider.is_none_or(|provider| {
                            provider.streaming(goodwin_plugin::provider::Capability::Speech)
                        })
                    });
        }
        self.settings.speech_streaming
    }

    pub(super) fn stream_speech(
        &self,
        answer: &str,
        consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Speech)? {
            return provider.stream_speech(answer, consume);
        }
        self.builtin.stream_speech(answer, consume)
    }
}

#[cfg(feature = "wasm-plugins")]
fn chat_request(
    instructions: &str,
    message: &str,
    history: &[Message],
) -> goodwin_plugin::provider::ChatRequest {
    use serde_json::json;
    let mut messages = vec![json!({"role": "system", "content": instructions})];
    messages.extend(
        history
            .iter()
            .rev()
            .map(|item| json!({"role": item.role, "content": item.text})),
    );
    messages.push(json!({"role": "user", "content": message}));
    goodwin_plugin::provider::ChatRequest {
        messages,
        tools: Vec::new(),
        tool_choice: None,
        max_completion_tokens: 256,
        selection: Default::default(),
    }
}

use goodwin_error::Result;

use crate::{
    builtin_assistant::{Provider, ProviderSettings},
    panel_core::Message,
    voice_core::VoiceRequest,
};

use super::http_client::HttpClient;
#[cfg(feature = "wasm-plugins")]
use crate::assistant::chat;

pub(super) struct Cascade<'a> {
    settings: &'a ProviderSettings,
    builtin: Provider<'a, HttpClient<'a>>,
    prompt: &'a str,
    #[cfg(feature = "wasm-plugins")]
    pub(super) plugins: Option<&'a super::plugins::Plugins>,
    #[cfg(feature = "wasm-plugins")]
    pub(super) assistant: Option<&'a crate::assistant::AssistantSettings>,
    #[cfg(feature = "wasm-plugins")]
    voice: Option<(
        &'a crate::voice_core::VoiceProviders<Option<super::plugins::ProviderBinding>>,
        &'a VoiceRequest,
    )>,
}

impl<'a> Cascade<'a> {
    pub(super) fn new(
        settings: &'a ProviderSettings,
        prompt: &'a str,
        request: Option<&'a VoiceRequest>,
    ) -> Self {
        Self {
            settings,
            builtin: Provider::new(settings, HttpClient(request)),
            prompt,
            #[cfg(feature = "wasm-plugins")]
            plugins: None,
            #[cfg(feature = "wasm-plugins")]
            assistant: None,
            #[cfg(feature = "wasm-plugins")]
            voice: None,
        }
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn bind_voice(
        mut self,
        bindings: &'a crate::voice_core::VoiceProviders<Option<super::plugins::ProviderBinding>>,
        request: &'a VoiceRequest,
    ) -> Self {
        self.voice = Some((bindings, request));
        self
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
            .ok_or_else(|| goodwin_error::report!("Plugin runtime is unavailable"))?;
        let binding = self
            .voice
            .map(|(bindings, request)| -> Result<_> {
                use goodwin_plugin::provider::Capability;
                let binding = match capability {
                    Capability::Transcription => &bindings.transcription,
                    Capability::Chat => &bindings.chat,
                    Capability::Speech => &bindings.speech,
                };
                Ok((
                    binding
                        .as_ref()
                        .ok_or_else(|| goodwin_error::report!("Voice provider is unavailable"))?,
                    request,
                ))
            })
            .transpose()?;
        Ok(Some(super::plugins::PluginProvider {
            plugin,
            settings: stage,
            binding,
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
        history: Vec<Message>,
        consume: impl FnMut(&str),
    ) -> Result<String> {
        let instructions = if self.prompt.is_empty() {
            crate::panel_core::CHAT_INSTRUCTIONS
        } else {
            self.prompt
        };
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            let mut request = chat::request(instructions, question, history);
            if provider.streaming(goodwin_plugin::provider::Capability::Chat) {
                return provider.stream_chat(&mut request, consume);
            }
            return chat::answer_text(provider.complete(&mut request)?.content.unwrap_or_default());
        }
        let _ = consume;
        self.builtin.chat(question, &history, instructions)
    }

    pub(super) fn reply(&self, text: &str, history: Vec<Message>) -> Result<String> {
        let instructions = self.voice_instructions();
        #[cfg(feature = "wasm-plugins")]
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            let mut request = chat::request(&instructions, text, history);
            return chat::answer_text(provider.complete(&mut request)?.content.unwrap_or_default());
        }
        self.builtin.chat(text, &history, &instructions)
    }

    pub(super) fn voice_instructions(&self) -> std::borrow::Cow<'_, str> {
        crate::voice_core::instructions(self.prompt)
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
impl crate::voice_core::tools::ToolChat for Cascade<'_> {
    async fn complete(
        &self,
        request: &mut goodwin_plugin::provider::ChatRequest,
    ) -> Result<goodwin_plugin::provider::ChatMessage> {
        if let Some(provider) = self.plugin(goodwin_plugin::provider::Capability::Chat)? {
            return provider.complete(request);
        }
        self.builtin.complete_tools(request)
    }
}

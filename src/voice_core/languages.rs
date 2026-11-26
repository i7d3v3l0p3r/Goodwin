use serde::{Deserialize, Deserializer, Serialize, de};

const LANGUAGE_CODES: &str = include_str!("../../shared/language-codes.txt");

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct LanguageCode(&'static str);

impl Default for LanguageCode {
    fn default() -> Self {
        Self("ru")
    }
}

impl<'de> Deserialize<'de> for LanguageCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        LANGUAGE_CODES
            .split_ascii_whitespace()
            .find(|&code| code == value)
            .map(Self)
            .ok_or_else(|| de::Error::custom("Expected an ISO 639-1 language code"))
    }
}

impl LanguageCode {
    pub(crate) fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AssistantLanguages {
    pub transcription: LanguageCode,
    pub reply: LanguageCode,
}

impl AssistantLanguages {
    fn instructions(self, prompt: &str, default: &str) -> String {
        let role = if prompt.is_empty() { default } else { prompt };
        format!(
            "{role}\n\nReply in the language with ISO 639-1 code \"{}\".",
            self.reply.as_str()
        )
    }

    pub(crate) fn chat_instructions(self, prompt: &str) -> String {
        self.instructions(
            prompt,
            "You are Goodwin, a home assistant. Reply concisely.",
        )
    }

    pub(crate) fn voice_instructions(self, prompt: &str) -> String {
        let mut instructions = self.instructions(prompt, "You are Goodwin, a voice assistant.");
        instructions.push_str(" Use concise plain text suitable for speaking aloud.");
        instructions
    }
}

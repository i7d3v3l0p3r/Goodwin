use serde::{Deserialize, Serialize};

pub(crate) const CHAT_INSTRUCTIONS: &str =
    "You are Goodwin, a home assistant. Reply concisely in the user's language.";

#[derive(Deserialize)]
pub(crate) struct UpdateMicrophone {
    pub muted: bool,
}

#[derive(Deserialize)]
pub(crate) struct UpdateSystemPrompt {
    pub system_prompt: String,
}

#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
#[derive(Deserialize)]
pub(crate) struct InstallWakeModel {
    pub(crate) id: String,
    pub(crate) tag: String,
}

#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
impl InstallWakeModel {
    pub(crate) fn valid(&self) -> bool {
        crate::wake_catalog::valid_model_id(&self.id) && self.tag.len() <= 33
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct OperationTicket {
    pub(crate) operation: String,
}

pub(crate) fn normalize_system_prompt(prompt: &str) -> goodwin_error::Result<String> {
    let prompt = prompt.trim().replace("\r\n", "\n").replace('\r', "\n");
    goodwin_error::ensure!(
        prompt.encode_utf16().count() <= 4_000,
        "System prompt exceeds 4,000 UTF-16 code units"
    );
    Ok(prompt)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(target_os = "espidf", allow(dead_code))]
pub enum Source {
    Chat,
    Wake,
    ManualTest,
    WakeTest,
    #[cfg(target_os = "espidf")]
    PushToTalk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(target_os = "espidf", allow(dead_code))]
pub enum Stage {
    AwaitingWake,
    Recording,
    Transcribing,
    Thinking,
    Synthesizing,
    Playing,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(target_os = "espidf", allow(dead_code))]
pub enum Failure {
    Cancelled,
    Busy,
    Microphone,
    MicrophoneMuted,
    Model,
    ProviderUnavailable,
    ProviderFailed,
    ChatInterrupted,
    Playback,
    NoSpeech,
    NoWake,
}

#[derive(Clone, Copy, Serialize)]
pub struct Timing {
    pub(crate) stage: Stage,
    pub(crate) ms: u64,
}

#[derive(Clone, Serialize)]
pub struct Report {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<u64>,
    pub(crate) source: Source,
    pub(crate) stage: Stage,
    pub(crate) timings: Vec<Timing>,
    pub(crate) error: Option<Failure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) chat_test: Option<ChatTest>,
    #[serde(skip)]
    #[cfg(not(target_os = "espidf"))]
    pub(crate) tested_configuration: Option<[u8; 32]>,
}

#[derive(Deserialize)]
pub(crate) struct CancelVoice {
    pub(crate) request_id: u64,
}

impl Report {
    pub(crate) fn record_timing(&mut self, elapsed: std::time::Duration) {
        self.timings.push(Timing {
            stage: self.stage,
            ms: elapsed.as_millis() as u64,
        });
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct ChatTest {
    pub(crate) model: VoiceModel,
    pub(crate) answered: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct VoiceModel {
    pub(crate) provider: String,
    pub(crate) model: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(target_os = "espidf", allow(dead_code))]
pub(crate) enum PlaybackMode {
    Streaming,
    Buffered,
    Announcement,
}

#[derive(Clone, Serialize)]
pub(crate) struct VoiceDetails {
    pub(crate) started_at_ms: u64,
    pub(crate) transcription: VoiceModel,
    pub(crate) chat: VoiceModel,
    pub(crate) speech: VoiceModel,
    pub(crate) transcript: Option<String>,
    pub(crate) answer: Option<String>,
    pub(crate) playback: Option<PlaybackMode>,
    pub(crate) error_stage: Option<Stage>,
}

#[derive(Clone, Serialize)]
pub(crate) struct VoiceReport {
    #[serde(flatten)]
    pub(crate) report: Report,
    #[serde(flatten)]
    pub(crate) details: VoiceDetails,
}

#[derive(Clone, Serialize)]
pub struct Message {
    pub(crate) id: u64,
    pub(crate) role: &'static str,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<Failure>,
}

pub(crate) fn recent_chat<'a>(newest_first: impl Iterator<Item = &'a Message>) -> Vec<Message> {
    newest_first
        .filter(|message| message.role != "assistant" || message.error.is_none())
        .take(12)
        .cloned()
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct AccessState {
    pub protected: bool,
    pub authorized: bool,
    pub setup_required: bool,
}

#[derive(Deserialize)]
pub(crate) struct SetupPassword {
    pub password: String,
    pub confirmation: String,
}

#[derive(Deserialize)]
pub(crate) struct Login {
    pub password: String,
}

#[derive(Deserialize)]
pub(crate) struct Chat {
    pub message: String,
    #[serde(default)]
    pub test: bool,
}

impl Chat {
    pub(crate) fn into_message(self) -> Option<String> {
        let message = self.message.trim();
        (!message.is_empty() && message.len() <= 8_000 && message.chars().count() <= 2_000)
            .then(|| message.to_owned())
    }
}

pub(crate) const SESSION_DURATION: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);

pub(crate) fn session_token(header: &str) -> Option<&str> {
    header
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == "goodwin_session").then_some(value))
}

pub(crate) fn session_cookie(token: &str, age: u64) -> String {
    format!("goodwin_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}")
}

pub(crate) fn same_origin(
    origin: Option<&str>,
    host: Option<&str>,
    fetch_site: Option<&str>,
) -> bool {
    fetch_site != Some("cross-site")
        && origin.is_none_or(|origin| {
            host.is_some()
                && origin
                    .strip_prefix("http://")
                    .or_else(|| origin.strip_prefix("https://"))
                    == host
        })
}

pub(crate) const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; font-src 'self' data:; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub(crate) const PAGE_PATHS: &[&str] = &[
    "/",
    #[cfg(not(target_os = "espidf"))]
    "/setup",
    #[cfg(not(target_os = "espidf"))]
    "/station",
    "/settings",
    "/settings/activation",
    "/settings/assistant",
    "/settings/diagnostics",
    "/settings/panel",
    #[cfg(target_os = "espidf")]
    "/settings/network",
    "/settings/update",
    #[cfg(not(target_os = "espidf"))]
    "/plugins",
];

pub(crate) const INDEX: (&str, &[u8]) = (
    "text/html; charset=utf-8",
    include_bytes!("../web/dist/index.html.gz"),
);

pub(crate) const ASSET_CACHE_CONTROL: &str = "public, max-age=0, must-revalidate";

pub(crate) struct Asset {
    pub path: &'static str,
    pub mime: &'static str,
    pub gzip: &'static [u8],
    pub etag: &'static str,
}

impl Asset {
    pub fn cached(&self, if_none_match: Option<&str>) -> bool {
        if_none_match.is_some_and(|header| {
            header
                .split(',')
                .map(str::trim)
                .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == self.etag)
        })
    }
}

macro_rules! embedded_asset {
    ($name:literal, $mime:literal) => {
        Asset {
            path: concat!("/", $name),
            mime: $mime,
            gzip: include_bytes!(concat!("../web/dist/", $name, ".gz")),
            etag: include_str!(concat!("../web/dist/", $name, ".etag")),
        }
    };
}

pub(crate) const ASSETS: &[Asset] = &[
    embedded_asset!("panel.js", "text/javascript; charset=utf-8"),
    embedded_asset!("plugin-runtime-v1.js", "text/javascript; charset=utf-8"),
    embedded_asset!("panel.css", "text/css; charset=utf-8"),
    embedded_asset!("panel-licenses.md", "text/plain; charset=utf-8"),
];

pub(crate) fn valid_password_length(password: &str) -> bool {
    (8..=256).contains(&password.chars().take(257).count())
}

pub(crate) fn random_bytes<const N: usize>() -> goodwin_error::Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes)
        .map_err(|_| goodwin_error::report!("Could not generate random bytes"))?;
    Ok(bytes)
}

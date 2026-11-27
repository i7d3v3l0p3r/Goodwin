use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
};

use esp_idf_svc::{
    http::{
        Method,
        server::{Configuration, EspHttpConnection, EspHttpServer, Request},
    },
    io::Write,
    sys,
};
use goodwin_error::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    builtin_assistant::{AudioSettings, ProviderUpdate, State},
    model,
    panel_core::{
        ASSET_CACHE_CONTROL, ASSETS, AccessState, CONTENT_SECURITY_POLICY, Chat, INDEX, Login,
        PAGE_PATHS, SetupPassword, Source, UpdateMicrophone, UpdateSystemPrompt, same_origin,
        session_cookie, session_token, valid_password_length,
    },
};

use super::{
    auth::Auth,
    firmware,
    network::{WifiCredentials, WifiState, WifiStatus},
    settings::Store,
    worker::VoiceJob,
};

mod jobs;
mod password_jobs;
#[cfg(feature = "wasm-plugins")]
mod plugins;
mod uploads;
#[cfg(feature = "wake-tflite")]
mod wake_models;

use crate::panel_core::OperationTicket;
use jobs::{Jobs, Outcome};
use password_jobs::Operation;

const BODY_LIMIT: usize = 16_384;

pub(super) struct Panel {
    pub state: Mutex<State>,
    credentials: Mutex<(Auth, Store)>,
    password_jobs: Jobs<String>,
    pub wifi: Mutex<WifiState>,
    pub reboot: AtomicBool,
    firmware_busy: AtomicBool,
    pub(super) clock_requested: AtomicBool,
    pub(super) clock_ready: AtomicBool,
    command: SyncSender<Command>,
    #[cfg(feature = "wake-tflite")]
    wake_catalog: bool,
    #[cfg(feature = "wake-tflite")]
    wake_model_busy: AtomicBool,
    #[cfg(feature = "wake-tflite")]
    wake_jobs: Jobs<wake_models::Completion>,
    #[cfg(feature = "wasm-plugins")]
    pub(super) plugins: std::sync::OnceLock<Arc<super::plugins::Plugins>>,
}

pub(super) enum Command {
    Chat(String),
    VoiceTest(VoiceJob),
    Correction(VoiceJob, String),
    FollowUp(VoiceJob, std::time::Instant),
    #[cfg(feature = "wake-tflite")]
    WakeTest(VoiceJob),
    #[cfg(feature = "wake-tflite")]
    UseWakeModel(
        super::wake_models::Candidate,
        std::sync::mpsc::Sender<goodwin_error::Result<crate::wake_catalog::Selection>>,
    ),
}

#[derive(Deserialize)]
struct UpdateAudio {
    settings: AudioSettings,
}

#[derive(Deserialize)]
enum TestMode {
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "wake")]
    Wake,
    #[serde(rename = "probe")]
    Probe,
}

#[derive(Deserialize)]
struct TestRequest {
    mode: TestMode,
}

impl Panel {
    pub fn new(
        store: Store,
        command: SyncSender<Command>,
        #[cfg(feature = "wake-tflite")] wake_model: Option<crate::wake_catalog::Selection>,
        #[cfg(feature = "wake-tflite")] wake_catalog: bool,
    ) -> Result<Self> {
        let provider = store.provider().unwrap_or_else(|error| {
            log::warn!("Invalid provider configuration: {error:#}");
            Default::default()
        });
        let audio = store.audio().unwrap_or_else(|error| {
            log::warn!("Invalid audio configuration: {error:#}");
            AudioSettings {
                microphone_muted: true,
                ..Default::default()
            }
        });
        let auth = Auth::new(store.password()?);
        let mut state = State::default();
        state.provider = provider;
        state.system_prompt = store.system_prompt().unwrap_or_else(|error| {
            log::warn!("Read system prompt: {error:#}");
            String::new()
        });
        #[cfg(feature = "wasm-plugins")]
        {
            state.assistant = store.assistant().unwrap_or_else(|error| {
                log::warn!("Read assistant settings: {error:#}");
                None
            });
        }
        state.audio = audio;
        #[cfg(feature = "wake-tflite")]
        {
            state.wake_model = wake_model;
        }
        Ok(Self {
            state: Mutex::new(state),
            credentials: Mutex::new((auth, store)),
            password_jobs: Jobs::default(),
            wifi: Mutex::new(WifiState::new()),
            reboot: AtomicBool::new(false),
            firmware_busy: AtomicBool::new(false),
            clock_requested: AtomicBool::new(false),
            clock_ready: AtomicBool::new(false),
            command,
            #[cfg(feature = "wake-tflite")]
            wake_catalog,
            #[cfg(feature = "wake-tflite")]
            wake_model_busy: AtomicBool::new(false),
            #[cfg(feature = "wake-tflite")]
            wake_jobs: Jobs::default(),
            #[cfg(feature = "wasm-plugins")]
            plugins: std::sync::OnceLock::new(),
        })
    }

    fn save_audio(&self, audio: AudioSettings) -> Result<()> {
        self.credentials
            .lock()
            .expect("panel credentials")
            .1
            .save_audio(audio)
    }

    pub fn save_wifi(&self, wifi: &WifiCredentials) -> Result<()> {
        self.credentials
            .lock()
            .expect("panel credentials")
            .1
            .save_wifi(wifi)
    }

    pub fn wifi_connected(&self) -> bool {
        matches!(
            self.wifi.lock().expect("Wi-Fi status").status,
            WifiStatus::Connected { .. }
        )
    }

    pub fn firmware_busy(&self) -> bool {
        self.firmware_busy.load(Ordering::Acquire)
    }

    pub(super) fn wake_model_busy(&self) -> bool {
        #[cfg(feature = "wake-tflite")]
        {
            self.wake_model_busy.load(Ordering::Acquire)
        }
        #[cfg(not(feature = "wake-tflite"))]
        {
            false
        }
    }

    #[cfg(feature = "wake-tflite")]
    pub(super) fn save_wake_selection(
        &self,
        selection: &crate::wake_catalog::Selection,
    ) -> Result<()> {
        self.credentials
            .lock()
            .expect("panel credentials")
            .1
            .save_wake_selection(selection)
    }

    #[cfg(feature = "wake-tflite")]
    pub(super) fn finish_wake_model_change(&self) {
        self.wake_model_busy.store(false, Ordering::Release);
    }

    pub(super) fn provider_ready(
        &self,
        state: &State,
        capability: goodwin_plugin::provider::Capability,
    ) -> bool {
        #[cfg(feature = "wasm-plugins")]
        if let Some(stage) = state
            .assistant
            .as_ref()
            .map(|settings| settings.pipeline.get(capability))
            && stage.provider != crate::builtin_assistant::BUILTIN_PROVIDER
        {
            return self
                .plugins
                .get()
                .is_some_and(|plugins| plugins.provider_available(&stage.provider, capability));
        }
        let _ = capability;
        state.provider.configured()
    }

    pub(super) fn voice_ready(&self, state: &State) -> bool {
        use goodwin_plugin::provider::Capability::*;
        [Transcription, Chat, Speech]
            .into_iter()
            .all(|capability| self.provider_ready(state, capability))
    }

    pub(super) fn start_follow_up(&self, state: &mut State) {
        if self.firmware_busy() || self.wake_model_busy() || !self.wifi_connected() {
            return;
        }
        if let Ok(job) = VoiceJob::admit(state, self, Source::FollowUp) {
            let request = job.request.clone();
            let command = Command::FollowUp(job, std::time::Instant::now());
            if crate::diagnostics::Queue::Device
                .send(&self.command, command)
                .is_err()
            {
                state.finish_voice(&request, Some(crate::panel_core::Failure::Busy));
            }
        }
    }

    fn access(&self, token: Option<&str>) -> AccessState {
        self.credentials
            .lock()
            .expect("panel credentials")
            .0
            .access(token)
    }
}

pub(super) fn serve(panel: Arc<Panel>) -> Result<uploads::Server> {
    let http = EspHttpServer::new(&Configuration {
        stack_size: if cfg!(feature = "wasm-plugins") {
            52 * 1024
        } else if cfg!(feature = "wake-tflite") {
            20 * 1024
        } else {
            12 * 1024
        },
        max_open_sockets: 5,
        uri_match_wildcard: true,
        ..Default::default()
    })?;
    let mut server = uploads::Server::new(http, panel.clone())?;
    for method in [
        Method::Get,
        Method::Post,
        Method::Put,
        #[cfg(feature = "wasm-plugins")]
        Method::Patch,
        #[cfg(feature = "wasm-plugins")]
        Method::Delete,
    ] {
        let panel = panel.clone();
        server
            .http
            .fn_handler("/*", method, move |request| handle(request, &panel))?;
    }
    Ok(server)
}

fn handle(mut request: Request<&mut EspHttpConnection<'_>>, panel: &Arc<Panel>) -> Result<()> {
    let method = request.method();
    let uri = request.uri();
    let path = uri.split('?').next().unwrap_or("/");
    if !path.starts_with("/api/") && !path.starts_with("/plugin-assets/") {
        if method == Method::Get {
            if let Some(asset) = ASSETS.iter().find(|asset| path == asset.path) {
                let cache = [("Cache-Control", ASSET_CACHE_CONTROL), ("ETag", asset.etag)];
                if asset.cached(request.header("If-None-Match")) {
                    return send(request, 304, asset.mime, &[], &cache);
                }
                let [policy, etag] = cache;
                return send(
                    request,
                    200,
                    asset.mime,
                    asset.gzip,
                    &[policy, etag, ("Content-Encoding", "gzip")],
                );
            }
            if path == "/"
                || PAGE_PATHS.contains(&path.trim_end_matches('/'))
                || (model::capabilities().plugin_packages
                    && (path == "/plugins" || path.starts_with("/plugins/")))
            {
                return send(
                    request,
                    200,
                    INDEX.0,
                    INDEX.1,
                    &[("Content-Encoding", "gzip")],
                );
            }
        }
        return problem(request, 404, "not_found");
    }
    if !same_origin(
        request.header("Origin"),
        request.header("Host"),
        request.header("Sec-Fetch-Site"),
    ) {
        return problem(request, 403, "origin");
    }
    let token = request
        .header("Cookie")
        .and_then(session_token)
        .map(str::to_owned);
    let access = panel.access(token.as_deref());
    match (method, path) {
        (Method::Get, "/api/capabilities") => {
            let capabilities = model::capabilities();
            let capabilities = model::Capabilities {
                #[cfg(feature = "wasm-plugins")]
                plugin_packages: panel.plugins.get().is_some(),
                #[cfg(feature = "wake-tflite")]
                wake_catalog: panel.wake_catalog,
                ..capabilities
            };
            return send_json(request, &capabilities);
        }
        (Method::Get, "/api/access") => return send_json(request, &access),
        (Method::Post, "/api/access/open") => {
            if read_json_body(&mut request)
                .and_then(|body| Ok(goodwin_json::validate(&body)?))
                .is_err()
            {
                return problem(request, 400, "invalid_request");
            }
            let mut credentials = panel.credentials.lock().expect("panel credentials");
            let access = credentials.0.access(token.as_deref());
            if !access.setup_required && !access.authorized {
                drop(credentials);
                return problem(request, 401, "login_required");
            }
            if access.protected {
                if let Err(error) = credentials.1.save_open_access() {
                    drop(credentials);
                    log::error!("Save open panel access: {error:#}");
                    return problem(request, 500, "access_save_failed");
                }
                credentials.0.set_open();
            }
            drop(credentials);
            return send_session(request, Ok(session_cookie("", 0)));
        }
        (Method::Post, "/api/setup" | "/api/access/password") => {
            let allowed = if path == "/api/setup" {
                access.setup_required
            } else {
                !access.protected
            };
            if !allowed {
                return problem(request, 409, "setup_complete");
            }
            let Ok(data) = read_json::<SetupPassword>(&mut request) else {
                return problem(request, 400, "invalid_password");
            };
            if !valid_password_length(&data.password) {
                return problem(request, 400, "invalid_password");
            }
            if data.password != data.confirmation {
                return problem(request, 400, "password_mismatch");
            }
            return send_ticket(
                request,
                password_jobs::submit(panel, Operation::Setup(data.password, access)),
            );
        }
        (Method::Post, "/api/login") => {
            if !access.protected {
                return send(request, 204, "text/plain", &[], &[]);
            }
            let Ok(data) = read_json::<Login>(&mut request) else {
                return problem(request, 400, "invalid_password");
            };
            return send_ticket(
                request,
                password_jobs::submit(panel, Operation::Login(data.password)),
            );
        }
        (Method::Post, "/api/auth/result") => {
            let Ok(ticket) = read_json::<OperationTicket>(&mut request) else {
                return problem(request, 400, "invalid_request");
            };
            let result = panel
                .password_jobs
                .take(&ticket.operation, "auth_result_expired");
            return match result {
                Ok(None) => send_pending(request),
                Ok(Some(hash)) => {
                    let mut credentials = panel.credentials.lock().expect("panel credentials");
                    if !credentials.0.uses_hash(&hash) {
                        drop(credentials);
                        return problem(request, 401, "wrong_password");
                    }
                    let cookie = credentials.0.create_session();
                    drop(credentials);
                    send_session(request, cookie)
                }
                Err((status, code)) => problem(request, status, code),
            };
        }
        (Method::Post, "/api/logout") => {
            panel
                .credentials
                .lock()
                .expect("panel credentials")
                .0
                .logout(token.as_deref());
            return send(
                request,
                204,
                "text/plain",
                &[],
                &[("Set-Cookie", &session_cookie("", 0))],
            );
        }
        _ => {}
    }
    if !access.authorized {
        return problem(request, 401, "login_required");
    }
    #[cfg(feature = "wasm-plugins")]
    if (path == "/api/plugins"
        || path.starts_with("/api/plugin")
        || path.starts_with("/plugin-assets/"))
        && let Some(plugins) = panel.plugins.get()
    {
        return plugins::handle(request, plugins, access.protected);
    }
    match (method, path) {
        #[cfg(feature = "wake-tflite")]
        (Method::Get | Method::Put, "/api/wake-models") => wake_models::handle(request, panel),
        #[cfg(feature = "wake-tflite")]
        (Method::Post, "/api/wake-models/result") => wake_models::result(request, panel),
        (Method::Get, "/api/network") => {
            let wifi = panel.wifi.lock().expect("Wi-Fi status").status.clone();
            send_json(request, &wifi)
        }
        (Method::Put, "/api/network") => {
            let Ok(next) =
                read_json::<WifiCredentials>(&mut request).and_then(WifiCredentials::validate)
            else {
                return problem(request, 400, "invalid_wifi");
            };
            let state = State::lock(&panel.state);
            let mut wifi = panel.wifi.lock().expect("Wi-Fi status");
            if state.busy()
                || panel.firmware_busy()
                || panel.wake_model_busy()
                || !wifi.schedule(next)
            {
                return problem(request, 409, "busy");
            }
            drop(wifi);
            drop(state);
            send(request, 202, "text/plain", &[], &[])
        }
        (Method::Get, "/api/firmware") => {
            if !access.protected {
                return problem(request, 403, "core_update_password_required");
            }
            if panel.firmware_busy() {
                return problem(request, 409, "busy");
            }
            match firmware::info() {
                Ok(info) => send_json(request, &info),
                Err(error) => {
                    log::warn!("Firmware info: {error:#}");
                    problem(request, 500, "firmware_update_failed")
                }
            }
        }
        (Method::Get, "/api/state") => {
            use goodwin_plugin::provider::Capability;
            let snapshot = {
                let state = State::lock(&panel.state);
                let mut snapshot = state.snapshot();
                snapshot["provider_ready"] = json!(panel.provider_ready(&state, Capability::Chat));
                snapshot["voice_provider_ready"] = json!(panel.voice_ready(&state));
                snapshot
            };
            #[cfg(feature = "wasm-plugins")]
            let snapshot = {
                let mut snapshot = snapshot;
                if let Some(plugins) = panel.plugins.get() {
                    snapshot["plugins"] = serde_json::to_value(plugins.catalog())?;
                }
                snapshot
            };
            send_json(request, &snapshot)
        }
        (Method::Put, "/api/settings") => {
            let Ok(update) = read_json::<UpdateAudio>(&mut request) else {
                return problem(request, 400, "invalid_settings");
            };
            let Ok(audio) = update.settings.validate() else {
                return problem(request, 400, "invalid_settings");
            };
            let mut state = State::lock(&panel.state);
            if state.busy() && !state.awaiting_speech() {
                return problem(request, 409, "busy");
            }
            let audio = AudioSettings {
                microphone_muted: state.audio.microphone_muted,
                ..audio
            };
            if panel.save_audio(audio).is_err() {
                return problem(request, 500, "save_failed");
            }
            state.audio = audio;
            drop(state);
            send(request, 204, "text/plain", &[], &[])
        }
        #[cfg(feature = "wasm-plugins")]
        (Method::Put, "/api/settings/assistant") => {
            let Ok(settings) = read_json_body(&mut request).and_then(|bytes| {
                crate::assistant::decode_settings::<crate::assistant::AssistantSettings>(&bytes)
            }) else {
                return problem(request, 400, "invalid_settings");
            };
            if settings.validate().is_err() {
                return problem(request, 400, "invalid_settings");
            }
            let mut state = State::lock(&panel.state);
            if state.busy() {
                return problem(request, 409, "busy");
            }
            if panel
                .credentials
                .lock()
                .expect("panel credentials")
                .1
                .save_assistant(&settings)
                .is_err()
            {
                return problem(request, 500, "save_failed");
            }
            state.assistant = Some(settings);
            send(request, 204, "text/plain", &[], &[])
        }
        (Method::Put, "/api/settings/microphone") => {
            let Ok(update) = read_json::<UpdateMicrophone>(&mut request) else {
                return problem(request, 400, "invalid_settings");
            };
            let mut state = State::lock(&panel.state);
            let audio = AudioSettings {
                microphone_muted: update.muted,
                ..state.audio
            };
            if panel.save_audio(audio).is_err() {
                return problem(request, 500, "save_failed");
            }
            state.audio = audio;
            if update.muted {
                state.mute_capture();
            }
            drop(state);
            send(request, 204, "text/plain", &[], &[])
        }
        (Method::Put, "/api/settings/system-prompt") => {
            let Ok(prompt) = read_json::<UpdateSystemPrompt>(&mut request).and_then(|update| {
                crate::panel_core::normalize_system_prompt(&update.system_prompt)
            }) else {
                return problem(request, 400, "invalid_system_prompt");
            };
            let mut state = State::lock(&panel.state);
            if panel
                .credentials
                .lock()
                .expect("panel credentials")
                .1
                .save_system_prompt(&prompt)
                .is_err()
            {
                return problem(request, 500, "save_failed");
            }
            state.system_prompt = prompt;
            send(request, 204, "text/plain", &[], &[])
        }
        (Method::Get, "/api/settings/provider") => {
            let view = State::lock(&panel.state).provider.view();
            send_json(request, &view)
        }
        (Method::Put, "/api/settings/provider") => {
            let Ok(update) = read_json::<ProviderUpdate>(&mut request) else {
                return problem(request, 400, "invalid_settings");
            };
            let mut state = State::lock(&panel.state);
            let Ok(next) = state.provider.update(update) else {
                return problem(request, 400, "invalid_settings");
            };
            if state.busy() {
                return problem(request, 409, "busy");
            }
            if panel
                .credentials
                .lock()
                .expect("panel credentials")
                .1
                .save_provider(&next)
                .is_err()
            {
                return problem(request, 500, "save_failed");
            }
            let view = next.view();
            state.provider = next;
            drop(state);
            send_json(request, &view)
        }
        (Method::Post, "/api/chat") => {
            let Ok(chat) = read_json::<Chat>(&mut request) else {
                return problem(request, 400, "invalid_message");
            };
            let test = chat.test;
            let Some(message) = chat.into_message() else {
                return problem(request, 400, "invalid_message");
            };
            let mut state = State::lock(&panel.state);
            if !panel.wifi_connected() {
                return problem(request, 409, "wifi_unavailable");
            }
            if !panel.provider_ready(&state, goodwin_plugin::provider::Capability::Chat) {
                return problem(request, 409, "provider_unavailable");
            }
            if state.busy()
                || panel.firmware_busy()
                || panel.wake_model_busy()
                || crate::diagnostics::Queue::Device
                    .send(&panel.command, Command::Chat(message.clone()))
                    .is_err()
            {
                return problem(request, 409, "busy");
            }
            state.start_chat(message, test);
            drop(state);
            send(request, 202, "text/plain", &[], &[])
        }
        (Method::Post, "/api/test") => {
            let Ok(test) = read_json::<TestRequest>(&mut request) else {
                return problem(request, 400, "invalid_message");
            };
            let mut state = State::lock(&panel.state);
            let source = match test.mode {
                TestMode::Manual => Source::ManualTest,
                #[cfg(feature = "wake-tflite")]
                TestMode::Wake if state.wake_loaded() => Source::WakeTest,
                #[cfg(feature = "wake-tflite")]
                TestMode::Probe if state.wake_loaded() => Source::WakeProbe,
                TestMode::Wake | TestMode::Probe => return problem(request, 409, "model"),
            };
            if state.audio.microphone_muted {
                return problem(request, 409, "microphone_muted");
            }
            if !panel.wifi_connected() {
                return problem(request, 409, "wifi_unavailable");
            }
            if state.busy() || panel.firmware_busy() || panel.wake_model_busy() {
                return problem(request, 409, "busy");
            }
            let Ok(job) = VoiceJob::admit(&mut state, panel, source) else {
                return problem(request, 409, "provider_unavailable");
            };
            let voice = job.request.clone();
            let command = match source {
                #[cfg(feature = "wake-tflite")]
                Source::WakeTest | Source::WakeProbe => Command::WakeTest(job),
                _ => Command::VoiceTest(job),
            };
            if crate::diagnostics::Queue::Device
                .send(&panel.command, command)
                .is_err()
            {
                state.finish_voice(&voice, Some(crate::panel_core::Failure::Busy));
                return problem(request, 409, "busy");
            }
            drop(state);
            send_json_status(request, 202, &json!({"request_id":voice.id}))
        }
        (Method::Post, "/api/voice/correct") => {
            let Ok(input) = read_json::<crate::panel_core::Correction>(&mut request) else {
                return problem(request, 400, "invalid_message");
            };
            if !input.valid() {
                return problem(request, 400, "invalid_message");
            }
            let mut state = State::lock(&panel.state);
            if state.busy()
                || panel.firmware_busy()
                || state.last_voice_id() != Some(input.request_id)
            {
                return problem(request, 409, "busy");
            }
            let Ok(job) = VoiceJob::admit(&mut state, panel, Source::Correction) else {
                return problem(request, 409, "provider_unavailable");
            };
            let voice = job.request.clone();
            state.transcript(input.text.clone());
            if crate::diagnostics::Queue::Device
                .send(&panel.command, Command::Correction(job, input.text))
                .is_err()
            {
                state.finish_voice(&voice, Some(crate::panel_core::Failure::Busy));
                return problem(request, 409, "busy");
            }
            drop(state);
            send_json_status(request, 202, &json!({"request_id":voice.id}))
        }
        (Method::Post, "/api/voice/cancel") => {
            let Ok(cancel) = read_json::<crate::panel_core::CancelVoice>(&mut request) else {
                return problem(request, 400, "invalid_message");
            };
            if State::lock(&panel.state).cancel_voice(cancel.request_id) {
                send(request, 202, "text/plain", &[], &[])
            } else {
                problem(request, 409, "voice_request_inactive")
            }
        }
        (Method::Post, "/api/chat/clear") => {
            let mut state = State::lock(&panel.state);
            if state.busy() {
                return problem(request, 409, "busy");
            }
            state.clear_chat();
            drop(state);
            send(request, 204, "text/plain", &[], &[])
        }
        (Method::Get, "/api/diagnostics/resources") => {
            #[cfg(any(feature = "wasm-plugins", feature = "wake-tflite"))]
            let storage = super::storage::usage();
            #[cfg(not(any(feature = "wasm-plugins", feature = "wake-tflite")))]
            let storage = None;
            send_json(request, &resources(storage))
        }
        (Method::Get, "/api/diagnostics/metrics") => {
            send_json(request, &crate::diagnostics::snapshot())
        }
        (Method::Get, "/api/diagnostics/logs") if !access.protected => {
            problem(request, 403, "diagnostics_password_required")
        }
        (Method::Get, "/api/diagnostics/logs") => send_json(
            request,
            &json!({
                "supported": true, "kernel": null, "goodwin": super::journal::recent(),
            }),
        ),
        _ => problem(request, 404, "not_found"),
    }
}

fn read_json<T: DeserializeOwned>(request: &mut Request<&mut EspHttpConnection<'_>>) -> Result<T> {
    Ok(goodwin_json::from_slice(&read_json_body(request)?)?)
}

fn read_json_body(request: &mut Request<&mut EspHttpConnection<'_>>) -> Result<Vec<u8>> {
    let content = request
        .header("Content-Type")
        .and_then(|header| header.split(';').next());
    goodwin_error::ensure!(content == Some("application/json"), "Expected JSON");
    let size: usize = request.header("Content-Length").unwrap_or("").parse()?;
    goodwin_error::ensure!(size <= BODY_LIMIT, "Request is too large");
    let mut bytes = vec![0; size];
    let mut offset = 0;
    while offset < size {
        let read = request.read(&mut bytes[offset..])?;
        goodwin_error::ensure!(read != 0, "Incomplete request");
        offset += read;
    }
    Ok(bytes)
}

fn send_session(
    request: Request<&mut EspHttpConnection<'_>>,
    cookie: Result<String>,
) -> Result<()> {
    match cookie {
        Ok(cookie) => send(request, 204, "text/plain", &[], &[("Set-Cookie", &cookie)]),
        Err(_) => problem(request, 500, "internal"),
    }
}

fn send_ticket(
    request: Request<&mut EspHttpConnection<'_>>,
    ticket: Outcome<String>,
) -> Result<()> {
    match ticket {
        Ok(operation) => send_json_status(request, 202, &OperationTicket { operation }),
        Err((status, code)) => problem(request, status, code),
    }
}

fn send_pending(request: Request<&mut EspHttpConnection<'_>>) -> Result<()> {
    send(
        request,
        202,
        "application/json",
        br#"{"pending":true}"#,
        &[],
    )
}

fn send_json(request: Request<&mut EspHttpConnection<'_>>, value: &impl Serialize) -> Result<()> {
    send_json_status(request, 200, value)
}

fn send_json_status(
    request: Request<&mut EspHttpConnection<'_>>,
    status: u16,
    value: &impl Serialize,
) -> Result<()> {
    send(
        request,
        status,
        "application/json",
        &goodwin_json::to_vec(value)?,
        &[],
    )
}

fn problem(request: Request<&mut EspHttpConnection<'_>>, status: u16, code: &str) -> Result<()> {
    send(
        request,
        status,
        "application/json",
        &goodwin_json::to_vec(&json!({"error": code}))?,
        &[],
    )
}

fn send(
    request: Request<&mut EspHttpConnection<'_>>,
    status: u16,
    mime: &str,
    content: &[u8],
    extra: &[(&str, &str)],
) -> Result<()> {
    let mut headers = vec![
        ("Content-Type", mime),
        ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
        ("X-Content-Type-Options", "nosniff"),
    ];
    if !extra.iter().any(|(name, _)| *name == "Cache-Control") {
        headers.push(("Cache-Control", "no-store"));
    }
    headers.extend_from_slice(extra);
    let mut response = request.into_response(status, Some(status_reason(status)), &headers)?;
    for block in content.chunks(4096) {
        response.write_all(block)?;
    }
    Ok(())
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

fn resources(storage: Option<serde_json::Value>) -> serde_json::Value {
    let memory = |region| {
        let caps = region | sys::MALLOC_CAP_8BIT;
        // SAFETY: These ESP-IDF accessors read the allocator counters for the given capabilities.
        let (total, free, minimum, largest) = unsafe {
            (
                sys::heap_caps_get_total_size(caps),
                sys::heap_caps_get_free_size(caps),
                sys::heap_caps_get_minimum_free_size(caps),
                sys::heap_caps_get_largest_free_block(caps),
            )
        };
        json!({
            "used_bytes": total.saturating_sub(free), "total_bytes": total,
            "minimum_free_bytes": minimum, "largest_free_bytes": largest,
        })
    };
    // SAFETY: The timer is initialized by ESP-IDF before app_main.
    let uptime = unsafe { sys::esp_timer_get_time() } / 1_000_000;
    json!({
        "supported": true, "cpu_percent": null, "storage": storage,
        "memory": memory(sys::MALLOC_CAP_INTERNAL), "psram": memory(sys::MALLOC_CAP_SPIRAM),
        "uptime_seconds": uptime, "goodwin_cpu_percent": null, "goodwin_rss_bytes": null,
    })
}

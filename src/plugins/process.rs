use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use goodwin_plugin::{
    provider::{
        CHAT_CHUNK_NOTIFICATION, CHAT_STREAM_METHOD, Capability, ChatRequest, MAX_CHAT_CHUNK_BYTES,
        MAX_RECORDING_BYTES, MAX_SPEECH_BYTES, MODELS_METHOD, ModelCatalog, ModelsRequest,
        SPEECH_CHUNK_NOTIFICATION, SPEECH_STREAM_METHOD, SpeechAudio, SpeechRequest,
    },
    wake::{EVENT_METHOD, Event, Observation},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
    time::timeout,
};

use super::package::{self, API_VERSION, Label, Manifest};

#[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
mod wasm;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_LIMIT: usize = 128 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub password_required: bool,
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
    pub parameters: Value,
}

impl Tool {
    fn valid(&self) -> bool {
        let Some(properties) = self.parameters["properties"].as_object() else {
            return false;
        };
        let Some(required) = self.parameters["required"].as_array() else {
            return false;
        };
        package::name(&self.name)
            && !self.description.trim().is_empty()
            && self.description.len() <= 1000
            && self.parameters["type"] == "object"
            && self.parameters["additionalProperties"] == false
            && properties.len() <= 16
            && required
                .iter()
                .all(|key| key.as_str().is_some_and(|key| properties.contains_key(key)))
            && properties.values().all(|property| {
                matches!(
                    property["type"].as_str(),
                    Some("string" | "integer" | "number" | "boolean")
                )
            })
    }

    pub fn accepts(&self, args: &Value) -> bool {
        let Some(args) = args.as_object() else {
            return false;
        };
        self.parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .all(|key| args.contains_key(key.as_str().unwrap()))
            && args.iter().all(|(key, value)| {
                match self.parameters["properties"][key]["type"].as_str() {
                    Some("string") => value.as_str().is_some_and(|value| value.len() <= 1000),
                    Some("integer") => value.is_i64() || value.is_u64(),
                    Some("number") => value.is_number(),
                    Some("boolean") => value.is_boolean(),
                    _ => false,
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
    chat_streaming: bool,
    #[serde(default)]
    pub(super) speech_streaming: bool,
    #[serde(default)]
    wake_events: bool,
    #[serde(default)]
    wake_subscription: u64,
}

impl Registration {
    fn validate(&self, manifest: &Manifest) -> Result<()> {
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

#[derive(Clone)]
pub struct Process {
    requests: mpsc::Sender<Call>,
    stop: watch::Sender<bool>,
    chat_streaming: bool,
    speech_streaming: bool,
    wake_subscription: Option<Arc<AtomicU64>>,
    provider_connection: Option<Arc<str>>,
}

struct Call {
    method: &'static str,
    params: Value,
    response: Option<oneshot::Sender<Result<Value>>>,
    chunks: Option<StreamChunks>,
    limits: CallLimits,
}

enum StreamChunks {
    Chat(mpsc::Sender<String>),
    Speech(mpsc::Sender<SpeechAudio>),
}

pub(crate) type SpeechReplies = StreamReplies<SpeechAudio>;

pub(crate) struct StreamReplies<T> {
    chunks: mpsc::Receiver<T>,
    result: Option<oneshot::Receiver<Result<Value>>>,
}

impl<T> StreamReplies<T> {
    pub(crate) async fn next(&mut self) -> Result<Option<T>> {
        if let Some(chunk) = self.chunks.recv().await {
            return Ok(Some(chunk));
        }
        if let Some(result) = self.result.take() {
            let result = result.await.context("Plugin process stopped")??;
            ensure!(result.is_null(), "Unexpected provider stream result");
        }
        Ok(None)
    }
}

#[derive(Clone, Copy)]
struct CallLimits {
    request: usize,
    response: usize,
    timeout: Duration,
}

impl CallLimits {
    const STANDARD: Self = Self {
        request: FRAME_LIMIT,
        response: FRAME_LIMIT,
        timeout: CALL_TIMEOUT,
    };

    fn provider(capability: Capability) -> Self {
        let (request, response) = match capability {
            Capability::Transcription => (
                MAX_RECORDING_BYTES.div_ceil(3) * 4 + FRAME_LIMIT,
                FRAME_LIMIT,
            ),
            Capability::Chat => (512 * 1024, 512 * 1024),
            Capability::Speech => (FRAME_LIMIT, MAX_SPEECH_BYTES.div_ceil(3) * 4 + FRAME_LIMIT),
        };
        Self {
            request,
            response,
            timeout: Duration::from_secs(60),
        }
    }
}

impl Process {
    #[cfg(test)]
    pub(crate) fn mock_chat_stream(mut input: mpsc::Receiver<Result<String>>) -> Self {
        let (requests, mut incoming) = mpsc::channel::<Call>(1);
        let (stop, mut stopping) = watch::channel(false);
        tokio::spawn(async move {
            let Some(call) = incoming.recv().await else {
                return;
            };
            assert_eq!(call.method, CHAT_STREAM_METHOD);
            let Some(StreamChunks::Chat(chunks)) = call.chunks else {
                panic!("Expected a chat stream")
            };
            let result = tokio::select! {
                _ = stopping.changed() => Err(anyhow!("Stopped")),
                result = async {
                    while let Some(chunk) = input.recv().await {
                        chunks.send(chunk?).await?;
                    }
                    Ok(Value::Null)
                } => result,
            };
            complete(call.response, result);
        });
        Self {
            requests,
            stop,
            chat_streaming: true,
            speech_streaming: false,
            wake_subscription: None,
            provider_connection: None,
        }
    }

    #[cfg(test)]
    pub fn mock(handler: impl Fn(Value) -> Value + Send + 'static) -> Self {
        Self::mock_rpc(move |method, params| {
            assert_eq!(method, "tool");
            Ok(handler(params))
        })
    }

    #[cfg(test)]
    pub fn mock_rpc(handler: impl Fn(&str, Value) -> Result<Value> + Send + 'static) -> Self {
        let (requests, mut incoming) = mpsc::channel::<Call>(1);
        let (stop, mut stopping) = watch::channel(false);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopping.changed() => break,
                    call = incoming.recv() => {
                        let Some(call) = call else { break; };
                        let result = handler(call.method, call.params);
                        if let Some(response) = call.response {
                            let _ = response.send(result);
                        }
                    }
                }
            }
        });
        Self {
            requests,
            stop,
            chat_streaming: false,
            speech_streaming: false,
            wake_subscription: None,
            provider_connection: None,
        }
    }

    pub async fn start(
        root: &Path,
        data: &Path,
        manifest: &Manifest,
        sound: mpsc::Sender<()>,
        catalog_changes: watch::Sender<()>,
    ) -> Result<(Self, Registration)> {
        if manifest.is_wasm() {
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            return wasm::start(root, data, manifest, sound, catalog_changes).await;
            #[cfg(not(all(feature = "wasm-plugins", target_os = "linux")))]
            bail!("This Goodwin build does not support WASM plugins");
        }
        let root = root.canonicalize()?;
        let data = data.canonicalize()?;
        let mut command = Command::new(root.join(&manifest.executable));
        command
            .current_dir(&data)
            .env_clear()
            .env("GOODWIN_PLUGIN_DATA", &data);
        #[cfg(windows)]
        if let Some(value) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Start plugin process")?;
        let mut input = child.stdin.take().context("Open plugin stdin")?;
        let mut output = Frames {
            reader: BufReader::new(child.stdout.take().context("Open plugin stdout")?),
            partial: Vec::new(),
        };
        let ready = async {
            let initialize = json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "apiVersion": API_VERSION, "chatStreaming": true, "speechStreaming": true, "wakeEvents": true, "pageIcons": true },
            });
            write(&mut input, &initialize, FRAME_LIMIT).await?;
            let ready: Registration =
                serde_json::from_value(response(output.read(FRAME_LIMIT).await?, 0)??)?;
            ready.validate(manifest)?;
            Ok::<_, anyhow::Error>(ready)
        };
        let registration = match timeout(START_TIMEOUT, ready)
            .await
            .context("Plugin readiness timed out")
            .and_then(|ready| ready)
        {
            Ok(registration) => registration,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let (requests, mut incoming) = mpsc::channel(8);
        let (stop, mut stopping) = watch::channel(false);
        let id = manifest.id.clone();
        let wake_subscription = registration
            .wake_events
            .then(|| Arc::new(AtomicU64::new(registration.wake_subscription)));
        let subscription = wake_subscription.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = stopping.changed() => {}
                result = run(&mut input, &mut output, &mut incoming, sound, subscription.as_deref()) => {
                    if let Err(error) = result { log::warn!("Plugin {id} stopped: {error}"); }
                }
            }
            drop(input);
            if timeout(Duration::from_millis(500), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
            drop(incoming);
            catalog_changes.send_replace(());
        });
        let speech_streaming = registration.speech_streaming;
        Ok((
            Self {
                requests,
                stop,
                chat_streaming: registration.chat_streaming,
                speech_streaming,
                wake_subscription,
                provider_connection: None,
            },
            registration,
        ))
    }

    pub fn running(&self) -> bool {
        !self.requests.is_closed() && !*self.stop.borrow()
    }

    pub(super) fn wake_subscription(&self) -> Option<u64> {
        let token = self.wake_subscription.as_ref()?.load(Ordering::Acquire);
        (token != 0 && self.running()).then_some(token)
    }

    pub(super) fn observe_wake(&self, subscription: u64, event: &Event) -> Result<()> {
        self.enqueue(Call {
            method: EVENT_METHOD,
            params: serde_json::to_value(Observation {
                subscription,
                event,
            })?,
            response: None,
            chunks: None,
            limits: CallLimits::STANDARD,
        })
    }

    pub(super) fn request_stop(&self) {
        let _ = self.stop.send(true);
    }

    pub async fn stop(&self) {
        self.request_stop();
        self.requests.closed().await;
    }

    pub async fn call(&self, method: &'static str, params: Value) -> Result<Value> {
        self.call_with_limits(method, params, CallLimits::STANDARD)
            .await
    }

    pub async fn provider(&self, capability: Capability, params: Value) -> Result<Value> {
        self.call_with_limits(
            capability.method(),
            params,
            CallLimits::provider(capability),
        )
        .await
    }

    pub(crate) fn stream_chat(
        &self,
        request: &ChatRequest,
    ) -> Result<Option<StreamReplies<String>>> {
        if !self.chat_streaming {
            return Ok(None);
        }
        let (sender, chunks) = mpsc::channel(8);
        let (response, result) = oneshot::channel();
        self.enqueue(Call {
            method: CHAT_STREAM_METHOD,
            params: serde_json::to_value(request)?,
            response: Some(response),
            chunks: Some(StreamChunks::Chat(sender)),
            limits: CallLimits {
                request: CallLimits::provider(Capability::Chat).request,
                response: FRAME_LIMIT,
                timeout: Duration::from_secs(60),
            },
        })?;
        Ok(Some(StreamReplies {
            chunks,
            result: Some(result),
        }))
    }

    pub(crate) fn stream_speech(&self, request: SpeechRequest) -> Result<Option<SpeechReplies>> {
        if !self.speech_streaming {
            return Ok(None);
        }
        let (sender, chunks) = mpsc::channel(4);
        let (response, result) = oneshot::channel();
        self.enqueue(Call {
            method: SPEECH_STREAM_METHOD,
            params: serde_json::to_value(request)?,
            response: Some(response),
            chunks: Some(StreamChunks::Speech(sender)),
            limits: CallLimits {
                timeout: Duration::from_secs(60),
                ..CallLimits::STANDARD
            },
        })?;
        Ok(Some(SpeechReplies {
            chunks,
            result: Some(result),
        }))
    }

    pub async fn models(&self, capability: Capability) -> Result<ModelCatalog> {
        let reply = self
            .call_with_limits(
                MODELS_METHOD,
                serde_json::to_value(ModelsRequest { capability })?,
                CallLimits {
                    request: FRAME_LIMIT,
                    response: 512 * 1024,
                    timeout: Duration::from_secs(5),
                },
            )
            .await?;
        Ok(serde_json::from_value(reply)?)
    }

    async fn call_with_limits(
        &self,
        method: &'static str,
        params: Value,
        limits: CallLimits,
    ) -> Result<Value> {
        let (response, result) = oneshot::channel();
        self.enqueue(Call {
            method,
            params,
            response: Some(response),
            chunks: None,
            limits,
        })?;
        timeout(limits.timeout * 2, result)
            .await
            .context("Plugin request timed out")?
            .context("Plugin process stopped")?
    }

    pub(super) fn for_connection(&self, id: &str, speech_streaming: bool) -> Self {
        let mut process = self.clone();
        process.provider_connection = Some(Arc::from(id));
        process.speech_streaming &= speech_streaming;
        process
    }

    fn enqueue(&self, mut call: Call) -> Result<()> {
        ensure!(self.running(), "Plugin process is unavailable");
        if let Some(id) = &self.provider_connection {
            call.params
                .as_object_mut()
                .context("Expected provider request object")?
                .insert("providerConnection".into(), Value::String(id.to_string()));
        }
        self.requests
            .try_send(call)
            .map_err(|_| anyhow!("Plugin process is busy or unavailable"))
    }
}

async fn run(
    input: &mut ChildStdin,
    output: &mut Frames,
    incoming: &mut mpsc::Receiver<Call>,
    sound: mpsc::Sender<()>,
    wake_subscription: Option<&AtomicU64>,
) -> Result<()> {
    let mut id = 0u64;
    loop {
        tokio::select! {
            frame = output.read(FRAME_LIMIT) => notify(frame?, &sound)?,
            call = incoming.recv() => {
                let Some(call) = call else { return Ok(()); };
                if call.response.as_ref().is_some_and(oneshot::Sender::is_closed) { continue; }
                id += 1;
                let message = json!({ "jsonrpc": "2.0", "id": id, "method": call.method, "params": call.params });
                let request = match encode(&message, call.limits.request) {
                    Ok(request) => request,
                    Err(error) => {
                        complete(call.response, Err(error));
                        continue;
                    }
                };
                let exchange = exchange(input, output, &sound, wake_subscription, id, &call, &request);
                let result = if matches!(call.chunks, Some(StreamChunks::Speech(_))) {
                    exchange.await?
                } else {
                    timeout(call.limits.timeout, exchange).await.context("Plugin RPC timed out")??
                };
                complete(call.response, result);
            }
        }
    }
}

fn complete(response: Option<oneshot::Sender<Result<Value>>>, result: Result<Value>) {
    if let Some(response) = response {
        let _ = response.send(result);
    } else if let Err(error) = result {
        log::warn!("Plugin wake event failed: {error}");
    }
}

async fn exchange(
    input: &mut ChildStdin,
    output: &mut Frames,
    sound: &mpsc::Sender<()>,
    wake_subscription: Option<&AtomicU64>,
    id: u64,
    call: &Call,
    request: &[u8],
) -> Result<Result<Value>> {
    timeout(call.limits.timeout, input.write_all(request))
        .await
        .context("Write plugin request timed out")??;
    loop {
        let message = timeout(call.limits.timeout, output.read(call.limits.response))
            .await
            .context("Read plugin response timed out")??;
        if message.get("id").is_some() {
            let subscription = message["wakeSubscription"].as_u64();
            let result = response(message, id)?;
            if let (Some(state), Some(subscription)) = (wake_subscription, subscription) {
                state.store(subscription, Ordering::Release);
            }
            return Ok(result);
        }
        match (&call.chunks, message["method"].as_str()) {
            (Some(StreamChunks::Speech(chunks)), Some(SPEECH_CHUNK_NOTIFICATION)) => {
                let SpeechChunk { request_id, audio } =
                    serde_json::from_value(message["params"].clone())
                        .context("Invalid plugin speech chunk")?;
                ensure!(
                    message["jsonrpc"] == "2.0" && request_id == id,
                    "Unexpected speech chunk"
                );
                // Keep draining RPC frames if the player has already stopped.
                let _ = chunks.send(audio).await;
            }
            (Some(StreamChunks::Chat(chunks)), Some(CHAT_CHUNK_NOTIFICATION)) => {
                let params = &message["params"];
                let text = params["text"]
                    .as_str()
                    .context("Missing plugin chat text")?;
                ensure!(
                    message["jsonrpc"] == "2.0"
                        && params["requestId"].as_u64() == Some(id)
                        && !text.is_empty()
                        && text.len() <= MAX_CHAT_CHUNK_BYTES,
                    "Invalid plugin chat chunk"
                );
                let _ = chunks.send(text.to_owned()).await;
            }
            _ => notify(message, sound)?,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechChunk {
    request_id: u64,
    audio: SpeechAudio,
}

fn response(mut message: Value, id: u64) -> Result<Result<Value>> {
    ensure!(
        message["jsonrpc"] == "2.0" && message["id"] == id,
        "Unexpected plugin RPC response"
    );
    if let Some(result) = message
        .as_object_mut()
        .and_then(|value| value.remove("result"))
    {
        return Ok(Ok(result));
    }
    let error = &message["error"];
    let code = error["code"].as_i64().context("Invalid plugin RPC error")?;
    let status = error["data"]["http_status"]
        .as_u64()
        .filter(|status| (100..=599).contains(status));
    Ok(Err(match status {
        Some(status) => anyhow!("Provider returned HTTP {status}"),
        None => anyhow!("Plugin rejected the request (RPC code {code})"),
    }))
}

fn notify(message: Value, sound: &mpsc::Sender<()>) -> Result<()> {
    ensure!(
        message["jsonrpc"] == "2.0"
            && message.get("id").is_none()
            && message["method"] == "play_timer_signal",
        "Unexpected plugin notification"
    );
    if sound.try_send(()).is_err() {
        log::warn!("Timer signal queue is full or unavailable");
    }
    Ok(())
}

fn encode(message: &Value, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(message)?;
    ensure!(bytes.len() < limit, "Plugin request is too large");
    bytes.push(b'\n');
    Ok(bytes)
}

async fn write(input: &mut ChildStdin, message: &Value, limit: usize) -> Result<()> {
    Ok(input.write_all(&encode(message, limit)?).await?)
}

struct Frames {
    reader: BufReader<ChildStdout>,
    partial: Vec<u8>,
}

impl Frames {
    async fn read(&mut self, limit: usize) -> Result<Value> {
        loop {
            let buffer = self.reader.fill_buf().await?;
            if buffer.is_empty() {
                bail!("Plugin closed stdout");
            }
            let newline = buffer.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(buffer.len(), |index| index + 1);
            ensure!(
                self.partial.len() + count <= limit,
                "Plugin response is too large"
            );
            self.partial.extend_from_slice(&buffer[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let message = serde_json::from_slice(&self.partial)?;
                self.partial.clear();
                return Ok(message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_details_accept_a_numeric_status_and_ignore_freeform_messages() {
        for (data, expected) in [
            (json!({"http_status": 429}), "Provider returned HTTP 429"),
            (
                json!({"http_status": "private text"}),
                "Plugin rejected the request (RPC code -32603)",
            ),
            (Value::Null, "Plugin rejected the request (RPC code -32603)"),
        ] {
            let error = response(
                json!({
                    "jsonrpc": "2.0", "id": 1,
                    "error": {"code": -32603, "message": "private text", "data": data}
                }),
                1,
            )
            .unwrap()
            .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }
}

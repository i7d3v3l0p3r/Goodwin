use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use goodwin_plugin::provider::{
    Capability, MAX_RECORDING_BYTES, MAX_SPEECH_BYTES, MODELS_METHOD, ModelCatalog, ModelsRequest,
    SPEECH_CHUNK_NOTIFICATION, SPEECH_STREAM_METHOD, SpeechAudio, SpeechRequest,
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

const START_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_LIMIT: usize = 128 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub method: String,
    pub path: String,
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
    pub(super) speech_streaming: bool,
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
    speech_streaming: bool,
}

struct Call {
    method: &'static str,
    params: Value,
    response: oneshot::Sender<Result<Value>>,
    speech_chunks: Option<mpsc::Sender<SpeechAudio>>,
    limits: CallLimits,
}

pub(crate) struct SpeechReplies {
    chunks: mpsc::Receiver<SpeechAudio>,
    result: Option<oneshot::Receiver<Result<Value>>>,
}

impl SpeechReplies {
    pub(crate) async fn next(&mut self) -> Result<Option<SpeechAudio>> {
        if let Some(audio) = self.chunks.recv().await {
            return Ok(Some(audio));
        }
        if let Some(result) = self.result.take() {
            let result = result.await.context("Plugin process stopped")??;
            ensure!(result.is_null(), "Unexpected speech stream result");
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
                        let _ = call.response.send(handler(call.method, call.params));
                    }
                }
            }
        });
        Self {
            requests,
            stop,
            speech_streaming: false,
        }
    }

    pub async fn start(
        root: &Path,
        data: &Path,
        manifest: &Manifest,
        sound: mpsc::Sender<()>,
    ) -> Result<(Self, Registration)> {
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
            write(&mut input, &json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "apiVersion": API_VERSION, "speechStreaming": true } }), FRAME_LIMIT).await?;
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
        tokio::spawn(async move {
            tokio::select! {
                _ = stopping.changed() => {}
                result = run(&mut input, &mut output, &mut incoming, sound) => {
                    if let Err(error) = result { eprintln!("Plugin {id} stopped: {error:#}"); }
                }
            }
            drop(input);
            if timeout(Duration::from_millis(500), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        });
        let speech_streaming = registration.speech_streaming;
        Ok((
            Self {
                requests,
                stop,
                speech_streaming,
            },
            registration,
        ))
    }

    pub fn running(&self) -> bool {
        !self.requests.is_closed() && !*self.stop.borrow()
    }

    pub async fn stop(&self) {
        let _ = self.stop.send(true);
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

    pub(crate) fn stream_speech(&self, request: SpeechRequest) -> Result<Option<SpeechReplies>> {
        if !self.speech_streaming {
            return Ok(None);
        }
        let (sender, chunks) = mpsc::channel(4);
        let (response, result) = oneshot::channel();
        self.enqueue(Call {
            method: SPEECH_STREAM_METHOD,
            params: serde_json::to_value(request)?,
            response,
            speech_chunks: Some(sender),
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
            response,
            speech_chunks: None,
            limits,
        })?;
        timeout(limits.timeout * 2, result)
            .await
            .context("Plugin request timed out")?
            .context("Plugin process stopped")?
    }

    fn enqueue(&self, call: Call) -> Result<()> {
        ensure!(self.running(), "Plugin process is unavailable");
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
) -> Result<()> {
    let mut id = 0u64;
    loop {
        tokio::select! {
            frame = output.read(FRAME_LIMIT) => notify(frame?, &sound)?,
            call = incoming.recv() => {
                let Some(call) = call else { return Ok(()); };
                if call.response.is_closed() { continue; }
                id += 1;
                let message = json!({ "jsonrpc": "2.0", "id": id, "method": call.method, "params": call.params });
                let request = match encode(&message, call.limits.request) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = call.response.send(Err(error));
                        continue;
                    }
                };
                let exchange = exchange(input, output, &sound, id, &call, &request);
                let result = if call.speech_chunks.is_some() {
                    exchange.await?
                } else {
                    timeout(call.limits.timeout, exchange).await.context("Plugin RPC timed out")??
                };
                let _ = call.response.send(result);
            }
        }
    }
}

async fn exchange(
    input: &mut ChildStdin,
    output: &mut Frames,
    sound: &mpsc::Sender<()>,
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
            return response(message, id);
        }
        if let Some(chunks) = &call.speech_chunks
            && message["method"] == SPEECH_CHUNK_NOTIFICATION
        {
            let SpeechChunk { request_id, audio } =
                serde_json::from_value(message["params"].clone())?;
            ensure!(
                message["jsonrpc"] == "2.0" && request_id == id,
                "Unexpected speech chunk"
            );
            // Keep draining RPC frames if the player has already stopped.
            let _ = chunks.send(audio).await;
        } else {
            notify(message, sound)?;
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
    ensure!(
        message["error"]["code"].is_i64(),
        "Invalid plugin RPC error"
    );
    Ok(Err(anyhow!("Plugin rejected the request")))
}

fn notify(message: Value, sound: &mpsc::Sender<()>) -> Result<()> {
    ensure!(
        message["jsonrpc"] == "2.0"
            && message.get("id").is_none()
            && message["method"] == "play_timer_signal",
        "Unexpected plugin notification"
    );
    if sound.try_send(()).is_err() {
        eprintln!("Timer signal queue is full or unavailable");
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

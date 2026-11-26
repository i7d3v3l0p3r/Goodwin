use std::{
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use goodwin_error::{Context, Result, bail, ensure, report};
use goodwin_plugin::{
    provider::{
        CHAT_CHUNK_NOTIFICATION, CHAT_STREAM_METHOD, Capability, ChatRequest, MAX_CHAT_CHUNK_BYTES,
        MAX_RECORDING_BYTES, MAX_SPEECH_BYTES, MODELS_METHOD, ModelCatalog, ModelsRequest,
        SPEECH_CHUNK_NOTIFICATION, SPEECH_STREAM_METHOD, SpeechAudio, SpeechRequest,
    },
    timer::{POLL_METHOD, Poll},
    wake::{EVENT_METHOD, Event, Observation},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{
    Value, json,
    value::{RawValue, to_raw_value},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
    time::timeout,
};

use super::package::{API_VERSION, Manifest};
use super::timer::{Delivery, Signal};
use crate::plugin_rpc;

pub(crate) use super::registration::{Page, Registration, Tool};

#[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
mod wasm;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_LIMIT: usize = 128 * 1024;
const SPEECH_DEADLINE: Duration = Duration::from_secs(180);
const SPEECH_DRAIN_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone)]
pub struct Process {
    requests: mpsc::Sender<Call>,
    stop: watch::Sender<bool>,
    chat_streaming: bool,
    speech_streaming: bool,
    wake_subscription: Option<Arc<AtomicU64>>,
    #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
    provider_connection: Option<ProviderTarget>,
}

#[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
#[derive(Clone)]
enum ProviderTarget {
    Connection(Arc<str>),
    Snapshot(super::ProviderConfig),
}

struct Call {
    method: &'static str,
    params: Box<RawValue>,
    #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
    connection: Option<ProviderTarget>,
    response: Option<CallResponse>,
    chunks: Option<StreamChunks>,
    limits: CallLimits,
}

impl Call {
    fn timer(poll: Poll) -> Result<Self> {
        Ok(Self {
            method: POLL_METHOD,
            params: to_raw_value(&poll)?,
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            connection: None,
            response: None,
            chunks: None,
            limits: CallLimits::STANDARD,
        })
    }
}

type CallResponse = oneshot::Sender<Result<Box<RawValue>>>;

enum StreamChunks {
    Chat(mpsc::Sender<String>),
    Speech(mpsc::Sender<SpeechAudio>),
}

pub(crate) type SpeechReplies = StreamReplies<SpeechAudio>;

pub(crate) struct StreamReplies<T> {
    chunks: mpsc::Receiver<T>,
    result: Option<oneshot::Receiver<Result<Box<RawValue>>>>,
}

impl<T> StreamReplies<T> {
    pub(crate) async fn next(&mut self) -> Result<Option<T>> {
        if let Some(chunk) = self.chunks.recv().await {
            return Ok(Some(chunk));
        }
        if let Some(result) = self.result.take() {
            let result = result.await.context("Plugin process stopped")??;
            ensure!(result.get() == "null", "Unexpected provider stream result");
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
                _ = stopping.changed() => Err(report!("Stopped")),
                result = async {
                    while let Some(chunk) = input.recv().await {
                        chunks.send(chunk?).await?;
                    }
                    Ok(to_raw_value(&())?)
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
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
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
                        let result = serde_json::from_str(call.params.get())
                            .map_err(goodwin_error::Error::from)
                            .and_then(|params| handler(call.method, params))
                            .and_then(|value| Ok(to_raw_value(&value)?));
                        complete(call.response, result);
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
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            provider_connection: None,
        }
    }

    pub async fn start(
        root: &Path,
        data: &Path,
        manifest: &Manifest,
        sound: mpsc::Sender<Signal>,
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
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let (mut native, registration, contract) =
            NativeProcess::start(&mut command, manifest).await?;
        let (requests, mut incoming) = mpsc::channel(8);
        let (stop, mut stopping) = watch::channel(false);
        let manifest = manifest.clone();
        let wake_subscription = registration
            .wake_events
            .then(|| Arc::new(AtomicU64::new(registration.wake_subscription)));
        let subscription = wake_subscription.clone();
        let timer_alarms = registration.timer_alarms;
        tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    _ = stopping.changed() => Ok(false),
                    result = run(&mut native.input, &mut native.output, &mut incoming, &sound, subscription.as_deref(), timer_alarms) => result,
                };
                native.close().await;
                match result {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        log::warn!("Plugin {} stopped: {error}", manifest.id);
                        break;
                    }
                }
                let restarted = tokio::select! {
                    _ = stopping.changed() => break,
                    result = NativeProcess::start(&mut command, &manifest) => result,
                };
                match restarted {
                    Ok((next, registration, new_contract)) if new_contract == contract => {
                        if let Some(subscription) = &subscription {
                            subscription.store(registration.wake_subscription, Ordering::Release);
                        }
                        native = next;
                    }
                    Ok((next, _, _)) => {
                        log::warn!(
                            "Plugin {} changed its registration after restart",
                            manifest.id
                        );
                        next.close().await;
                        break;
                    }
                    Err(error) => {
                        log::warn!("Could not restart plugin {}: {error:#}", manifest.id);
                        break;
                    }
                }
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
                #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
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
            params: to_raw_value(&Observation {
                subscription,
                event,
            })?,
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            connection: None,
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
        let payload = to_raw_value(&params)?;
        drop(params);
        let reply = self
            .call_with_limits(method, payload, CallLimits::STANDARD)
            .await?;
        Ok(serde_json::from_str(reply.get())?)
    }

    pub async fn provider<T: DeserializeOwned>(
        &self,
        capability: Capability,
        params: impl Serialize,
    ) -> Result<T> {
        let payload = to_raw_value(&params)?;
        drop(params);
        let reply = self
            .call_with_limits(
                capability.method(),
                payload,
                CallLimits::provider(capability),
            )
            .await?;
        Ok(goodwin_json::from_str(reply.get())?)
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
            params: to_raw_value(request)?,
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            connection: self.provider_connection.clone(),
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
            params: to_raw_value(&request)?,
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            connection: self.provider_connection.clone(),
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
                to_raw_value(&ModelsRequest { capability })?,
                CallLimits {
                    request: FRAME_LIMIT,
                    response: 512 * 1024,
                    timeout: Duration::from_secs(5),
                },
            )
            .await?;
        Ok(goodwin_json::from_str(reply.get())?)
    }

    async fn call_with_limits(
        &self,
        method: &'static str,
        params: Box<RawValue>,
        limits: CallLimits,
    ) -> Result<Box<RawValue>> {
        let (response, result) = oneshot::channel();
        self.enqueue(Call {
            method,
            params,
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            connection: self.provider_connection.clone(),
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
        #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
        {
            process.provider_connection = Some(ProviderTarget::Connection(Arc::from(id)));
        }
        #[cfg(not(all(feature = "wasm-plugins", target_os = "linux")))]
        let _ = id;
        process.speech_streaming &= speech_streaming;
        process
    }

    #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
    pub(super) fn with_provider_config(mut self, config: super::ProviderConfig) -> Self {
        if let super::ProviderConfig::Connection(connection) = &config {
            self.speech_streaming &= connection.protocol.speech_streaming();
        }
        self.provider_connection = Some(ProviderTarget::Snapshot(config));
        self
    }

    fn enqueue(&self, call: Call) -> Result<()> {
        ensure!(self.running(), "Plugin process is unavailable");
        self.requests.try_send(call).map_err(|error| {
            if matches!(error, mpsc::error::TrySendError::Full(_)) {
                crate::diagnostics::Queue::Plugin.full();
            }
            report!("Plugin process is busy or unavailable")
        })
    }
}

struct NativeProcess {
    child: Child,
    input: ChildStdin,
    output: Frames,
}

impl NativeProcess {
    async fn start(
        command: &mut Command,
        manifest: &Manifest,
    ) -> Result<(Self, Registration, Value)> {
        let mut child = command.spawn().context("Start plugin process")?;
        let mut input = child.stdin.take().context("Open plugin stdin")?;
        let mut output = Frames {
            reader: BufReader::new(child.stdout.take().context("Open plugin stdout")?),
            partial: Vec::new(),
        };
        let ready = async {
            let initialize = json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "apiVersion": API_VERSION, "chatStreaming": true, "speechStreaming": true, "wakeEvents": true, "timerAlarms": true },
            });
            write(&mut input, &initialize, FRAME_LIMIT).await?;
            let reply = decode_reply(output.read(FRAME_LIMIT).await?, 0)??;
            let registration: Registration = serde_json::from_str(reply.get())?;
            registration.validate(manifest)?;
            let mut contract: Value = serde_json::from_str(reply.get())?;
            if let Some(fields) = contract.as_object_mut() {
                fields.remove("wakeSubscription");
            }
            Ok::<_, goodwin_error::Error>((registration, contract))
        };
        let (registration, contract) = match timeout(START_TIMEOUT, ready)
            .await
            .context("Plugin readiness timed out")
            .and_then(|result| result)
        {
            Ok(ready) => ready,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        Ok((
            Self {
                child,
                input,
                output,
            },
            registration,
            contract,
        ))
    }

    async fn close(self) {
        let Self {
            mut child,
            input,
            output,
        } = self;
        drop(input);
        drop(output);
        if timeout(Duration::from_millis(500), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

async fn run(
    input: &mut ChildStdin,
    output: &mut Frames,
    incoming: &mut mpsc::Receiver<Call>,
    sound: &mpsc::Sender<Signal>,
    wake_subscription: Option<&AtomicU64>,
    timer_alarms: bool,
) -> Result<bool> {
    let mut id = 0u64;
    let mut timer = Delivery::new(timer_alarms);
    loop {
        let mut call = tokio::select! {
            frame = output.read(FRAME_LIMIT) => {
                notify(frame?, sound)?;
                continue;
            }
            call = incoming.recv() => {
                let Some(call) = call else { return Ok(false); };
                call
            }
            poll = timer.wait(), if timer.enabled() => Call::timer(poll)?,
        };
        if call
            .response
            .as_ref()
            .is_some_and(oneshot::Sender::is_closed)
        {
            continue;
        }
        id += 1;
        let message = plugin_rpc::Request {
            jsonrpc: "2.0",
            id,
            method: call.method,
            params: &call.params,
            now: None,
            timer_clock: None,
        };
        let request = match encode(&message, call.limits.request) {
            Ok(request) => request,
            Err(error) => {
                complete(call.response, Err(error));
                continue;
            }
        };
        let mut response = call.response.take();
        let exchange = exchange(input, output, sound, wake_subscription, id, &call, &request);
        let result = if let Some(StreamChunks::Speech(chunks)) = &call.chunks {
            match finish_speech_exchange(exchange, chunks).await {
                Ok(result) => result,
                Err(error) => {
                    log::warn!("Restarting native speech plugin: {error:#}");
                    complete(response, Err(error));
                    return Ok(true);
                }
            }
        } else {
            tokio::select! {
                biased;
                result = timeout(call.limits.timeout, exchange) => result.context("Plugin RPC timed out")??,
                () = response_closed(response.as_mut()) => {
                    // A cancelled exchange can leave a partial frame in either pipe.
                    return Ok(true);
                }
            }
        };
        if call.method == POLL_METHOD {
            timer.update(
                result.and_then(|reply| Ok(serde_json::from_str(reply.get())?)),
                sound,
            );
        } else {
            timer.changed();
            complete(response, result);
        }
    }
}

async fn response_closed(response: Option<&mut CallResponse>) {
    if let Some(response) = response {
        response.closed().await;
    } else {
        std::future::pending::<()>().await;
    }
}

async fn finish_speech_exchange<T>(
    exchange: impl Future<Output = Result<T>>,
    chunks: &mpsc::Sender<SpeechAudio>,
) -> Result<T> {
    timeout(SPEECH_DEADLINE, async {
        let mut exchange = std::pin::pin!(exchange);
        tokio::select! {
            biased;
            result = &mut exchange => result,
            _ = chunks.closed() => timeout(SPEECH_DRAIN_TIMEOUT, exchange)
                .await
                .context("Plugin speech did not finish after cancellation")?,
        }
    })
    .await
    .context("Plugin speech deadline exceeded")?
}

fn complete(response: Option<CallResponse>, result: Result<Box<RawValue>>) {
    if let Some(response) = response {
        let _ = response.send(result);
    } else if let Err(error) = result {
        log::warn!("Plugin wake event failed: {error}");
    }
}

async fn exchange(
    input: &mut ChildStdin,
    output: &mut Frames,
    sound: &mpsc::Sender<Signal>,
    wake_subscription: Option<&AtomicU64>,
    id: u64,
    call: &Call,
    request: &[u8],
) -> Result<Result<Box<RawValue>>> {
    timeout(call.limits.timeout, input.write_all(request))
        .await
        .context("Write plugin request timed out")??;
    loop {
        let message = timeout(call.limits.timeout, output.read(call.limits.response))
            .await
            .context("Read plugin response timed out")??;
        if message.id.is_some() {
            let subscription = message.wake_subscription;
            let result = decode_reply(message, id)?;
            if let (Some(state), Some(subscription)) = (wake_subscription, subscription) {
                state.store(subscription, Ordering::Release);
            }
            return Ok(result);
        }
        match (&call.chunks, message.method.as_deref()) {
            (Some(StreamChunks::Speech(chunks)), Some(SPEECH_CHUNK_NOTIFICATION)) => {
                let SpeechChunk { request_id, audio } = serde_json::from_str(
                    message
                        .params
                        .as_deref()
                        .context("Missing plugin speech parameters")?
                        .get(),
                )
                .context("Invalid plugin speech chunk")?;
                ensure!(
                    message.jsonrpc == "2.0" && request_id == id,
                    "Unexpected speech chunk"
                );
                // The dispatcher bounds this drain when playback has stopped.
                let _ = chunks.send(audio).await;
            }
            (Some(StreamChunks::Chat(chunks)), Some(CHAT_CHUNK_NOTIFICATION)) => {
                let ChatChunk { request_id, text } = serde_json::from_str(
                    message
                        .params
                        .as_deref()
                        .context("Missing plugin chat parameters")?
                        .get(),
                )?;
                ensure!(
                    message.jsonrpc == "2.0"
                        && request_id == id
                        && !text.is_empty()
                        && text.len() <= MAX_CHAT_CHUNK_BYTES,
                    "Invalid plugin chat chunk"
                );
                let _ = chunks.send(text).await;
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatChunk {
    request_id: u64,
    text: String,
}

type WireReply = plugin_rpc::Response<Box<RawValue>>;

fn decode_reply(message: WireReply, id: u64) -> Result<Result<Box<RawValue>>> {
    ensure!(
        message.jsonrpc == "2.0" && message.id == Some(id),
        "Unexpected plugin RPC response"
    );
    if let Some(result) = message.result {
        return Ok(Ok(result));
    }
    let error = message.error.context("Invalid plugin RPC error")?;
    let code = error.code;
    let status = error
        .data
        .get("http_status")
        .and_then(Value::as_u64)
        .filter(|status| (100..=599).contains(status));
    Ok(Err(match status {
        Some(status) => report!("Provider returned HTTP {status}"),
        None => report!("Plugin rejected the request (RPC code {code})"),
    }))
}

#[cfg(test)]
fn response(message: Value, id: u64) -> Result<Result<Value>> {
    let message = serde_json::from_slice(&serde_json::to_vec(&message)?)?;
    Ok(decode_reply(message, id)?.and_then(|value| Ok(serde_json::from_str(value.get())?)))
}

fn notify(message: WireReply, sound: &mpsc::Sender<Signal>) -> Result<()> {
    ensure!(
        message.jsonrpc == "2.0"
            && message.id.is_none()
            && message.method.as_deref() == Some("play_timer_signal"),
        "Unexpected plugin notification"
    );
    if sound.try_send(Signal::default()).is_err() {
        log::warn!("Timer signal queue is full or unavailable");
    }
    Ok(())
}

fn encode(message: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
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
    async fn read(&mut self, limit: usize) -> Result<WireReply> {
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

    #[tokio::test]
    async fn abandoned_speech_can_finish_at_the_current_rpc_boundary() {
        let (chunks, consumer) = mpsc::channel(1);
        drop(consumer);
        let result = finish_speech_exchange(
            async {
                tokio::task::yield_now().await;
                Ok("complete")
            },
            &chunks,
        )
        .await
        .unwrap();
        assert_eq!(result, "complete");
    }

    #[tokio::test]
    async fn abandoned_speech_cannot_hold_the_worker_waiting_for_a_reply() {
        let (chunks, consumer) = mpsc::channel(1);
        drop(consumer);
        let error = finish_speech_exchange(std::future::pending::<Result<()>>(), &chunks)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("after cancellation"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_restarted_native_plugin_serves_the_next_queued_request() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("fixture");
        fs::write(
            &executable,
            format!(
                r#"#!/bin/sh
set -eu
IFS= read -r request
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"apiVersion":{API_VERSION},"speechStreaming":true}}}}'
IFS= read -r request
if [ ! -f speech-started ]; then
    : > speech-started
    IFS= read -r request
    exit 1
fi
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":"restarted"}}'
IFS= read -r request
"#
            ),
        )
        .unwrap();
        fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();

        let manifest = serde_json::from_value(json!({
            "id": "fixture", "title": {"en":"Fixture","ru":"Fixture"},
            "version": "1.0.0", "apiVersion": API_VERSION,
            "target": "linux-aarch64", "executable": "fixture",
        }))
        .unwrap();
        let (sound, _signals) = mpsc::channel(1);
        let (changes, _catalog) = watch::channel(());
        let (plugin, _) = Process::start(root.path(), root.path(), &manifest, sound, changes)
            .await
            .unwrap();
        let speech = plugin
            .stream_speech(SpeechRequest {
                text: "Hello".into(),
                selection: Default::default(),
            })
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), async {
            while !root.path().join("speech-started").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        drop(speech);

        let reply = timeout(
            Duration::from_secs(5),
            plugin.call_with_limits(
                "health",
                to_raw_value(&json!({})).unwrap(),
                CallLimits::STANDARD,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.get(), r#""restarted""#);
    }

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

#![cfg_attr(all(target_arch = "wasm32", not(feature = "wasm-guest")), no_std)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(all(target_arch = "wasm32", not(feature = "wasm-guest")))]
use alloc::string::String;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(any(not(target_arch = "wasm32"), feature = "plugin-api"))]
pub use goodwin_error as error;
#[cfg(any(not(target_arch = "wasm32"), feature = "plugin-api"))]
use goodwin_error::Result;
#[cfg(not(target_arch = "wasm32"))]
use goodwin_error::{Context, ensure};
#[cfg(not(target_arch = "wasm32"))]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(any(not(target_arch = "wasm32"), feature = "plugin-api"))]
use serde_json::json;

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-guest"))]
pub mod audio;
pub use goodwin_json as json_codec;
pub mod provider;
#[doc(hidden)]
pub mod rpc;
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-guest"))]
pub mod services;
#[cfg(not(target_arch = "wasm32"))]
pub mod settings;
pub mod timer;
pub mod wake;

#[cfg(not(target_arch = "wasm32"))]
use provider::{
    Capability, MODELS_METHOD, ProviderHttpError, SPEECH_CHUNK_NOTIFICATION, SPEECH_STREAM_METHOD,
};
#[cfg(any(not(target_arch = "wasm32"), feature = "plugin-api"))]
use provider::{
    ChatRequest, Completion, ModelCatalog, ModelsRequest, SpeechAudio, SpeechRequest,
    Transcription, TranscriptionRequest,
};

#[cfg(any(not(target_arch = "wasm32"), feature = "plugin-api"))]
pub trait Plugin {
    fn registration(&self) -> Value;

    #[cfg(not(target_arch = "wasm32"))]
    fn ready(&mut self, _host: Host) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn supports_timer_alarms(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_timer(&mut self, _poll: timer::Poll) -> Result<timer::Status> {
        goodwin_error::bail!("Timer alarms are unavailable")
    }

    fn http(&mut self, _request: HttpRequest) -> Result<HttpResponse> {
        Ok(HttpResponse::json(404, json!({"error": "not_found"})))
    }

    fn supports_wake_events(&self) -> bool {
        false
    }

    fn wake_event(&mut self, _event: wake::Observation) -> Result<()> {
        goodwin_error::bail!("Wake events are unavailable")
    }

    fn wake_subscription(&self) -> u64 {
        0
    }

    fn tool(&mut self, _call: ToolCall) -> Result<Value> {
        Ok(json!({"error": "unknown_tool"}))
    }

    fn transcribe(&mut self, _request: TranscriptionRequest) -> Result<Transcription> {
        goodwin_error::bail!("Transcription is unavailable")
    }

    fn complete(&mut self, _request: ChatRequest) -> Result<Completion> {
        goodwin_error::bail!("Chat completion is unavailable")
    }

    fn supports_chat_streaming(&self) -> bool {
        false
    }

    fn complete_stream(
        &mut self,
        _request: ChatRequest,
        _emit: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        goodwin_error::bail!("Chat streaming is unavailable")
    }

    fn synthesize(&mut self, _request: SpeechRequest) -> Result<SpeechAudio> {
        goodwin_error::bail!("Speech synthesis is unavailable")
    }

    fn supports_speech_streaming(&self) -> bool {
        false
    }

    fn synthesize_stream(
        &mut self,
        _request: SpeechRequest,
        _emit: &mut dyn FnMut(SpeechAudio) -> Result<()>,
    ) -> Result<()> {
        goodwin_error::bail!("Speech streaming is unavailable")
    }

    fn models(&mut self, _request: ModelsRequest) -> Result<ModelCatalog> {
        goodwin_error::bail!("The model catalog is unavailable")
    }
}

#[derive(Deserialize, Serialize)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub body: Value,
}

#[derive(Serialize)]
pub struct HttpResponse {
    status: u16,
    body: Option<Value>,
}

impl HttpResponse {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: Some(body),
        }
    }

    pub fn empty(status: u16) -> Self {
        Self { status, body: None }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    pub voice_turn: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Host(Arc<Mutex<Output>>);

#[cfg(not(target_arch = "wasm32"))]
type SendFrame = dyn FnMut(&[u8]) -> Result<()> + Send;

#[cfg(not(target_arch = "wasm32"))]
struct Output {
    send: Box<SendFrame>,
    pending_signals: Option<usize>,
    timer_alarms: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Host {
    pub fn timer_alarms(&self) -> bool {
        self.0.lock().expect("plugin output").timer_alarms
    }

    pub fn play_timer_signal(&self) -> Result<()> {
        let mut output = self.0.lock().expect("plugin output");
        if let Some(count) = &mut output.pending_signals {
            *count += 1;
            return Ok(());
        }
        output.send_timer_signal()
    }

    fn confirm_ready(&self, reply: &Value) -> Result<()> {
        let mut output = self.0.lock().expect("plugin output");
        output.send(reply)?;
        for _ in 0..output.pending_signals.take().expect("startup signals") {
            output.send_timer_signal()?;
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Output {
    fn send(&mut self, frame: &impl Serialize) -> Result<()> {
        let frame = serde_json::to_vec(frame)?;
        (self.send)(&frame)
    }

    fn send_timer_signal(&mut self) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": "play_timer_signal"}))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(plugin: impl Plugin) -> Result<()> {
    run_protocol(plugin, io::stdin().lock(), send)
}

#[cfg(not(target_arch = "wasm32"))]
fn run_protocol(
    mut plugin: impl Plugin,
    input: impl BufRead,
    send: impl FnMut(&[u8]) -> Result<()> + Send + 'static,
) -> Result<()> {
    use rpc::Payload;
    let mut input = input.lines();
    let line = input
        .next()
        .context("Expected the Goodwin initialize request")??;
    let init = rpc::Frame::read(line.as_bytes())?;
    let params: rpc::Initialize = init.params()?;
    ensure!(
        init.method == "initialize" && params.api_version == 1,
        "Unsupported Goodwin protocol"
    );
    let timer_alarms = params.timer_alarms && plugin.supports_timer_alarms();
    let host = Host(Arc::new(Mutex::new(Output {
        send: Box::new(send),
        pending_signals: Some(0),
        timer_alarms,
    })));
    plugin.ready(host.clone());
    let speech_streaming = params.speech_streaming && plugin.supports_speech_streaming();
    let chat_streaming = params.chat_streaming && plugin.supports_chat_streaming();
    let wake_events = params.wake_events && plugin.supports_wake_events();
    let mut registration = plugin.registration();
    if timer_alarms {
        registration["timerAlarms"] = json!(true);
    }
    if speech_streaming {
        registration["speechStreaming"] = json!(true);
    }
    if chat_streaming {
        registration["chatStreaming"] = json!(true);
    }
    if wake_events {
        registration["wakeEvents"] = json!(true);
        registration["wakeSubscription"] = json!(plugin.wake_subscription());
    }
    host.confirm_ready(&json!({"jsonrpc": "2.0", "id": init.id, "result": registration}))?;

    for line in input {
        let line = line?;
        let request = rpc::Frame::read(line.as_bytes())?;
        let previous_subscription = wake_events.then(|| plugin.wake_subscription());
        let result = match request.method.as_str() {
            timer::POLL_METHOD if timer_alarms => dispatch(&request, |poll| {
                Ok(Payload::Json(serde_json::to_value(
                    plugin.poll_timer(poll)?,
                )?))
            }),
            "http" => dispatch(&request, |request| plugin.http(request).map(Payload::Http)),
            "tool" => request
                .params()
                .map_err(goodwin_error::Error::from)
                .and_then(|call| plugin.tool(call).map(Payload::Json)),
            method if method == Capability::Transcription.method() => {
                dispatch(&request, |request| {
                    plugin.transcribe(request).map(Payload::Transcription)
                })
            }
            method if method == Capability::Chat.method() => dispatch(&request, |request| {
                plugin.complete(request).map(Payload::Chat)
            }),
            provider::CHAT_STREAM_METHOD if chat_streaming => dispatch(&request, |chat| {
                plugin
                    .complete_stream(chat, &mut |text| {
                        ensure!(
                            !text.is_empty() && text.len() <= provider::MAX_CHAT_CHUNK_BYTES,
                            "Invalid chat chunk length"
                        );
                        host.0
                            .lock()
                            .expect("plugin output")
                            .send(&rpc::Notification {
                                jsonrpc: "2.0",
                                method: provider::CHAT_CHUNK_NOTIFICATION,
                                params: rpc::ChatChunk {
                                    request_id: request.id,
                                    text,
                                },
                            })
                    })
                    .map(Payload::Empty)
            }),
            method if method == Capability::Speech.method() => dispatch(&request, |request| {
                plugin.synthesize(request).map(Payload::Speech)
            }),
            SPEECH_STREAM_METHOD if speech_streaming => dispatch(&request, |speech| {
                plugin
                    .synthesize_stream(speech, &mut |audio| {
                        host.0
                            .lock()
                            .expect("plugin output")
                            .send(&rpc::Notification {
                                jsonrpc: "2.0",
                                method: SPEECH_CHUNK_NOTIFICATION,
                                params: rpc::SpeechChunk {
                                    request_id: request.id,
                                    audio,
                                },
                            })
                    })
                    .map(Payload::Empty)
            }),
            MODELS_METHOD => dispatch(&request, |request| {
                plugin.models(request).map(Payload::Models)
            }),
            wake::EVENT_METHOD if wake_events => dispatch(&request, |event| {
                plugin.wake_event(event).map(Payload::Empty)
            }),
            _ => Err(goodwin_error::report!("Unknown RPC method")),
        };
        let (result, error) = match result {
            Ok(result) => (Some(result), None),
            Err(error) => {
                eprintln!("Plugin request: {error:#}");
                let mut reply = json!({"code": -32603, "message": "Plugin request failed"});
                if let Some(status) = error.find::<ProviderHttpError>() {
                    reply["data"] = json!({"http_status": status.0});
                }
                (None, Some(reply))
            }
        };
        let wake_subscription = previous_subscription
            .map(|previous| (previous, plugin.wake_subscription()))
            .and_then(|(previous, current)| (previous != current).then_some(current));
        host.0.lock().expect("plugin output").send(&rpc::Reply {
            jsonrpc: "2.0",
            id: request.id,
            result,
            error,
            wake_subscription,
        })?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch<T: DeserializeOwned>(
    frame: &rpc::Frame<'_>,
    handler: impl FnOnce(T) -> Result<rpc::Payload>,
) -> Result<rpc::Payload> {
    handler(frame.params()?)
}

#[cfg(not(target_arch = "wasm32"))]
fn send(message: &[u8]) -> Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(message)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn data_dir() -> Result<PathBuf> {
    std::env::var_os("GOODWIN_PLUGIN_DATA")
        .map(PathBuf::from)
        .context("GOODWIN_PLUGIN_DATA is missing")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_json<T: Default + DeserializeOwned>(path: &Path) -> Result<T> {
    match fs::File::open(path) {
        Ok(file) => Ok(serde_json::from_reader(file)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path
        .parent()
        .context("Data file must be inside a directory")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, value)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn run_protocol(
        plugin: impl Plugin,
        input: impl BufRead,
        mut send: impl FnMut(&Value) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        super::run_protocol(plugin, input, move |frame| {
            send(&serde_json::from_slice(frame)?)
        })
    }

    const INITIALIZE: &[u8] =
        br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"apiVersion":1}}"#;

    struct StartupPlugin {
        initialized: bool,
        fail: bool,
    }

    impl Plugin for StartupPlugin {
        fn registration(&self) -> Value {
            assert!(self.initialized);
            json!({"apiVersion": 1})
        }

        fn ready(&mut self, host: Host) {
            host.play_timer_signal().unwrap();
            assert!(!self.fail, "startup failed");
            self.initialized = true;
        }
    }

    #[test]
    fn startup_signal_follows_the_successful_initialize_response() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&frames);
        let plugin = StartupPlugin {
            initialized: false,
            fail: false,
        };
        run_protocol(plugin, INITIALIZE, move |frame| {
            captured.lock().unwrap().push(frame.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            *frames.lock().unwrap(),
            [
                json!({"jsonrpc": "2.0", "id": 0, "result": {"apiVersion": 1}}),
                json!({"jsonrpc": "2.0", "method": "play_timer_signal"}),
            ]
        );
    }

    #[test]
    fn failed_initialization_sends_neither_readiness_nor_a_startup_signal() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&frames);
        let plugin = StartupPlugin {
            initialized: false,
            fail: true,
        };
        let failure = std::panic::catch_unwind(|| {
            run_protocol(plugin, INITIALIZE, move |frame| {
                captured.lock().unwrap().push(frame.clone());
                Ok(())
            })
        });

        assert!(failure.is_err());
        assert!(frames.lock().unwrap().is_empty());
    }

    #[test]
    fn initialization_preserves_page_icons() {
        struct PagesPlugin;

        impl Plugin for PagesPlugin {
            fn registration(&self) -> Value {
                json!({"apiVersion": 1, "pages": [{"id": "main", "navigation": true, "icon": "timer"}]})
            }
        }

        run_protocol(PagesPlugin, INITIALIZE, |frame| {
            let page = &frame["result"]["pages"][0];
            assert_eq!(page["id"], "main");
            assert_eq!(page["navigation"], true);
            assert_eq!(page["icon"], "timer");
            Ok(())
        })
        .unwrap();
    }

    struct SpeechPlugin;

    impl Plugin for SpeechPlugin {
        fn registration(&self) -> Value {
            json!({"apiVersion": 1, "providers": ["speech"]})
        }

        fn supports_speech_streaming(&self) -> bool {
            true
        }

        fn synthesize(&mut self, _request: SpeechRequest) -> Result<SpeechAudio> {
            Err(goodwin_error::Error::from(ProviderHttpError(429))
                .context("Upstream request failed"))
        }

        fn synthesize_stream(
            &mut self,
            request: SpeechRequest,
            emit: &mut dyn FnMut(SpeechAudio) -> Result<()>,
        ) -> Result<()> {
            assert_eq!(request.text, "Готово");
            emit(SpeechAudio {
                pcm_base64: "AAE=".into(),
                sample_rate: 24_000,
                channels: 1,
            })
        }
    }

    #[test]
    fn provider_failures_report_the_http_status_to_the_host() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&frames);
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"apiVersion":1}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"provider.synthesize","params":{"text":"Готово"}}"#,
        );
        run_protocol(SpeechPlugin, input.as_bytes(), move |frame| {
            captured.lock().unwrap().push(frame.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            frames.lock().unwrap()[1]["error"],
            json!({"code": -32603, "message": "Plugin request failed", "data": {"http_status": 429}})
        );
    }

    #[test]
    fn speech_chunks_precede_the_result_and_are_advertised_only_to_streaming_hosts() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let legacy = Arc::clone(&frames);
        run_protocol(SpeechPlugin, INITIALIZE, move |frame| {
            legacy.lock().unwrap().push(frame.clone());
            Ok(())
        })
        .unwrap();

        let input = concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"apiVersion":1,"speechStreaming":true}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":5,"method":"provider.synthesize_stream","params":{"text":"Готово"}}"#,
            "\n",
        );
        let modern = Arc::clone(&frames);
        run_protocol(SpeechPlugin, input.as_bytes(), move |frame| {
            modern.lock().unwrap().push(frame.clone());
            Ok(())
        })
        .unwrap();
        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 4);
        assert!(frames[0]["result"].get("speechStreaming").is_none());
        assert_eq!(frames[1]["result"]["speechStreaming"], true);
        assert_eq!(
            frames[2],
            json!({
                "jsonrpc": "2.0", "method": "provider.speech_chunk",
                "params": {"requestId":5,"audio":{"pcmBase64":"AAE=","sampleRate":24000,"channels":1}}
            })
        );
        assert_eq!(frames[3], json!({"jsonrpc":"2.0","id":5,"result":null}));
    }

    #[test]
    fn chat_chunks_are_negotiated_and_keep_the_request_id() {
        struct ChatPlugin;
        impl Plugin for ChatPlugin {
            fn registration(&self) -> Value {
                json!({"apiVersion": 1, "providers": ["chat"]})
            }

            fn supports_chat_streaming(&self) -> bool {
                true
            }

            fn complete_stream(
                &mut self,
                _request: ChatRequest,
                emit: &mut dyn FnMut(&str) -> Result<()>,
            ) -> Result<()> {
                emit("Готово")?;
                emit("!")
            }
        }

        for enabled in [false, true] {
            let frames = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&frames);
            let input = format!(
                "{}\n{}\n",
                json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"apiVersion": 1, "chatStreaming": enabled}}),
                json!({"jsonrpc": "2.0", "id": 7, "method": "provider.complete_stream", "params": {"messages": [], "max_completion_tokens": 256}}),
            );
            run_protocol(ChatPlugin, input.as_bytes(), move |frame| {
                captured.lock().unwrap().push(frame.clone());
                Ok(())
            })
            .unwrap();
            let frames = frames.lock().unwrap();
            if enabled {
                assert_eq!(frames[0]["result"]["chatStreaming"], true);
                for (frame, text) in frames[1..3].iter().zip(["Готово", "!"]) {
                    assert_eq!(
                        *frame,
                        json!({
                            "jsonrpc": "2.0", "method": "provider.chat_chunk",
                            "params": {"requestId": 7, "text": text}
                        })
                    );
                }
                assert_eq!(
                    frames[3],
                    json!({"jsonrpc": "2.0", "id": 7, "result": null})
                );
            } else {
                assert!(frames[0]["result"].get("chatStreaming").is_none());
                assert_eq!(frames.len(), 2);
                assert_eq!(frames[1]["error"]["code"], -32603);
            }
        }
    }
}

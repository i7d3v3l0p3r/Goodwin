use std::{cell::RefCell, io};

use goodwin_error::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::Request;
use crate::{HttpRequest, Plugin, ToolCall, provider, rpc, wake};

thread_local! {
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[link(wasm_import_module = "goodwin")]
unsafe extern "C" {
    fn host_call(meta: *const u8, meta_len: usize, data: *const u8, data_len: usize) -> i32;
    fn host_result(output: *mut u8, length: usize) -> i32;
}

pub fn call(request: Request, input: &[u8], output: &mut [u8]) -> Result<usize> {
    let length = response_length(request, input, output.len())?;
    read_response(&mut output[..length])?;
    Ok(length)
}

/// Reads a host response into a vector sized for the returned bytes.
pub fn call_vec(request: Request, input: &[u8], limit: usize) -> Result<Vec<u8>> {
    let length = response_length(request, input, limit)?;
    let mut output = Vec::new();
    output.try_reserve_exact(length)?;
    output.resize(length, 0);
    read_response(&mut output)?;
    Ok(output)
}

fn response_length(request: Request, input: &[u8], limit: usize) -> Result<usize> {
    let meta = crate::json_codec::to_vec(&request)?;
    // The import borrows these buffers only for the duration of this call.
    let length = unsafe { host_call(meta.as_ptr(), meta.len(), input.as_ptr(), input.len()) };
    let length = usize::try_from(length).context("Goodwin host operation failed")?;
    ensure!(length <= limit, "Goodwin host response exceeds its limit");
    Ok(length)
}

fn read_response(output: &mut [u8]) -> Result<()> {
    if !output.is_empty() {
        ensure!(
            unsafe { host_result(output.as_mut_ptr(), output.len()) } == 0,
            "Read Goodwin host response"
        );
    }
    Ok(())
}

pub fn read_settings<T: DeserializeOwned>() -> Result<T> {
    Ok(serde_json::from_slice(&call_vec(
        Request::Settings,
        &[],
        32 * 1024,
    )?)?)
}

/// Maximum total size of guest data files, in bytes. Requires `wasmStorage`.
pub fn storage_limit() -> Result<u64> {
    let mut output = [0; 20];
    let used = call(Request::StorageLimit, &[], &mut output)?;
    Ok(crate::json_codec::from_slice(&output[..used])?)
}

pub fn http_read(buffer: &mut [u8]) -> io::Result<usize> {
    call(
        Request::HttpRead {
            length: buffer.len().min(64 * 1024),
        },
        &[],
        buffer,
    )
    .map_err(|error| io::Error::other(error.into_boxed()))
}

pub fn input(length: usize) -> *mut u8 {
    BUFFER.with_borrow_mut(|buffer| {
        *buffer = vec![0; length];
        buffer.as_mut_ptr()
    })
}

pub fn buffer() -> *mut u8 {
    BUFFER.with_borrow_mut(Vec::as_mut_ptr)
}

pub fn dispatch<P: Plugin>(plugin: &mut Option<P>, length: usize, open: fn() -> Result<P>) -> i32 {
    let input = BUFFER.with_borrow_mut(std::mem::take);
    let request = rpc::Frame::read(input.get(..length).unwrap_or_default());
    let Ok(request) = request else { return -1 };
    let id = request.id;
    let call = Call::read(&request);
    drop(input);
    let previous = plugin.as_ref().map_or(0, Plugin::wake_subscription);
    let result = call.and_then(|call| execute(plugin, call, open));
    let (result, error) = match result {
        Ok(result) => (Some(result), None),
        Err(error) => {
            let mut error_reply = json!({"code":-32603, "message":"Plugin request failed"});
            if let Some(status) = error.find::<provider::ProviderHttpError>() {
                error_reply["data"] = json!({"http_status":status.0});
            }
            (None, Some(error_reply))
        }
    };
    let subscription = plugin.as_ref().map_or(0, Plugin::wake_subscription);
    let reply = rpc::Reply {
        jsonrpc: "2.0",
        id,
        result,
        error,
        wake_subscription: (subscription != previous).then_some(subscription),
    };
    // Replies can contain full buffered audio or an arbitrary plugin
    // response. The streaming serializer handles long strings in one pass.
    let Ok(output) = serde_json::to_vec(&reply) else {
        return -1;
    };
    let length = output.len() as i32;
    BUFFER.with_borrow_mut(|buffer| *buffer = output);
    length
}

enum Call {
    Initialize(rpc::Initialize),
    Http(HttpRequest),
    Tool(ToolCall),
    Wake(wake::Observation),
    Models(provider::ModelsRequest),
    ChatStream(provider::ChatRequest),
    SpeechStream(provider::SpeechRequest),
    Transcribe(provider::TranscriptionRequest),
    Chat(provider::ChatRequest),
    Speech(provider::SpeechRequest),
}

impl Call {
    fn read(frame: &rpc::Frame<'_>) -> Result<Self> {
        Ok(match frame.method.as_str() {
            "initialize" => Self::Initialize(frame.params()?),
            "http" => Self::Http(frame.params()?),
            "tool" => Self::Tool(frame.params()?),
            wake::EVENT_METHOD => Self::Wake(frame.params()?),
            provider::MODELS_METHOD => Self::Models(frame.params()?),
            provider::CHAT_STREAM_METHOD => Self::ChatStream(frame.params()?),
            provider::SPEECH_STREAM_METHOD => Self::SpeechStream(frame.params()?),
            method if method == provider::Capability::Transcription.method() => {
                Self::Transcribe(frame.params()?)
            }
            method if method == provider::Capability::Chat.method() => Self::Chat(frame.params()?),
            method if method == provider::Capability::Speech.method() => {
                Self::Speech(frame.params()?)
            }
            _ => goodwin_error::bail!("Unknown plugin method"),
        })
    }
}

fn execute<P: Plugin>(
    plugin: &mut Option<P>,
    request: Call,
    open: fn() -> Result<P>,
) -> Result<rpc::Payload> {
    use rpc::Payload;
    if let Call::Initialize(params) = request {
        ensure!(
            plugin.is_none() && params.api_version == 1,
            "Invalid initialize request"
        );
        let opened = open()?;
        let mut registration = opened.registration();
        if opened.supports_chat_streaming() {
            registration["chatStreaming"] = json!(true);
        }
        if opened.supports_speech_streaming() {
            registration["speechStreaming"] = json!(true);
        }
        if opened.supports_wake_events() {
            registration["wakeEvents"] = json!(true);
            registration["wakeSubscription"] = json!(opened.wake_subscription());
        }
        *plugin = Some(opened);
        return Ok(Payload::Json(registration));
    }
    let plugin = plugin.as_mut().context("Plugin is not initialized")?;
    Ok(match request {
        Call::Initialize(_) => unreachable!(),
        Call::Http(request) => Payload::Http(plugin.http(request)?),
        Call::Tool(request) => Payload::Json(plugin.tool(request)?),
        Call::Wake(request) => {
            plugin.wake_event(request)?;
            Payload::Empty(())
        }
        Call::Models(request) => Payload::Models(plugin.models(request)?),
        Call::ChatStream(request) => {
            plugin.complete_stream(request, &mut |chunk| {
                call(Request::Chat, chunk.as_bytes(), &mut [])?;
                Ok(())
            })?;
            Payload::Empty(())
        }
        Call::SpeechStream(request) => {
            plugin.synthesize_stream(request, &mut |chunk| {
                call(
                    Request::Speech,
                    &crate::json_codec::to_vec(&chunk)?,
                    &mut [],
                )?;
                Ok(())
            })?;
            Payload::Empty(())
        }
        Call::Transcribe(request) => Payload::Transcription(plugin.transcribe(request)?),
        Call::Chat(request) => Payload::Chat(plugin.complete(request)?),
        Call::Speech(request) => Payload::Speech(plugin.synthesize(request)?),
    })
}

#[macro_export]
macro_rules! wasm_plugin {
    ($plugin:ty, $open:path) => {
        std::thread_local! {
            static PLUGIN: std::cell::RefCell<Option<$plugin>> = const { std::cell::RefCell::new(None) };
        }

        #[unsafe(no_mangle)]
        extern "C" fn goodwin_input(length: usize) -> *mut u8 {
            $crate::services::guest::input(length)
        }

        #[unsafe(no_mangle)]
        extern "C" fn goodwin_buffer() -> *mut u8 {
            $crate::services::guest::buffer()
        }

        #[unsafe(no_mangle)]
        extern "C" fn goodwin_call(length: usize) -> i32 {
            PLUGIN.with_borrow_mut(|plugin| $crate::services::guest::dispatch(plugin, length, $open))
        }
    };
}

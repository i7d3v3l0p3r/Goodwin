use std::{cell::RefCell, io};

use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::Request;
use crate::{Plugin, provider, wake};

thread_local! {
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[link(wasm_import_module = "goodwin")]
unsafe extern "C" {
    fn host_call(meta: *const u8, meta_len: usize, data: *const u8, data_len: usize) -> i32;
    fn host_result(output: *mut u8, length: usize) -> i32;
}

pub fn call(request: Request, input: &[u8], output: &mut [u8]) -> Result<usize> {
    let meta = serde_json::to_vec(&request)?;
    // The import borrows these buffers only for the duration of this call.
    let length = unsafe { host_call(meta.as_ptr(), meta.len(), input.as_ptr(), input.len()) };
    let length = usize::try_from(length).context("Goodwin host operation failed")?;
    ensure!(
        length <= output.len(),
        "Goodwin host response exceeds its limit"
    );
    if length != 0 {
        ensure!(
            unsafe { host_result(output.as_mut_ptr(), length) } == 0,
            "Read Goodwin host response"
        );
    }
    Ok(length)
}

pub fn read_settings<T: DeserializeOwned>() -> Result<T> {
    let mut buffer = vec![0; 32 * 1024];
    let used = call(Request::Settings, &[], &mut buffer)?;
    Ok(serde_json::from_slice(&buffer[..used])?)
}

pub fn http_read(buffer: &mut [u8]) -> io::Result<usize> {
    call(
        Request::HttpRead {
            length: buffer.len().min(64 * 1024),
        },
        &[],
        buffer,
    )
    .map_err(io::Error::other)
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
    let request = BUFFER.with_borrow_mut(|buffer| {
        let buffer = std::mem::take(buffer);
        serde_json::from_slice::<Frame>(buffer.get(..length).unwrap_or_default())
    });
    let Ok(request) = request else { return -1 };
    let id = request.id;
    let previous = plugin.as_ref().map_or(0, Plugin::wake_subscription);
    let result = execute(plugin, request, open);
    let mut reply = match result {
        Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
        Err(error) => {
            let mut error_reply = json!({"code":-32603, "message":"Plugin request failed"});
            if let Some(status) = error.downcast_ref::<provider::ProviderHttpError>() {
                error_reply["data"] = json!({"http_status":status.0});
            }
            json!({"jsonrpc":"2.0", "id":id, "error":error_reply})
        }
    };
    let subscription = plugin.as_ref().map_or(0, Plugin::wake_subscription);
    if subscription != previous {
        reply["wakeSubscription"] = json!(subscription);
    }
    let Ok(output) = serde_json::to_vec(&reply) else {
        return -1;
    };
    let length = output.len() as i32;
    BUFFER.with_borrow_mut(|buffer| *buffer = output);
    length
}

#[derive(serde::Deserialize)]
struct Frame {
    id: u64,
    method: String,
    params: Value,
}

fn execute<P: Plugin>(
    plugin: &mut Option<P>,
    frame: Frame,
    open: fn() -> Result<P>,
) -> Result<Value> {
    if frame.method == "initialize" {
        ensure!(
            plugin.is_none() && frame.params["apiVersion"] == 1,
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
        return Ok(registration);
    }
    let plugin = plugin.as_mut().context("Plugin is not initialized")?;
    let params = frame.params;
    Ok(match frame.method.as_str() {
        "http" => serde_json::to_value(plugin.http(serde_json::from_value(params)?)?)?,
        "tool" => plugin.tool(serde_json::from_value(params)?)?,
        wake::EVENT_METHOD => {
            plugin.wake_event(serde_json::from_value(params)?)?;
            Value::Null
        }
        provider::MODELS_METHOD => {
            serde_json::to_value(plugin.models(serde_json::from_value(params)?)?)?
        }
        provider::CHAT_STREAM_METHOD => {
            plugin.complete_stream(serde_json::from_value(params)?, &mut |chunk| {
                call(Request::Chat, chunk.as_bytes(), &mut [])?;
                Ok(())
            })?;
            Value::Null
        }
        provider::SPEECH_STREAM_METHOD => {
            plugin.synthesize_stream(serde_json::from_value(params)?, &mut |chunk| {
                call(Request::Speech, &serde_json::to_vec(&chunk)?, &mut [])?;
                Ok(())
            })?;
            Value::Null
        }
        method if method == provider::Capability::Transcription.method() => {
            serde_json::to_value(plugin.transcribe(serde_json::from_value(params)?)?)?
        }
        method if method == provider::Capability::Chat.method() => {
            serde_json::to_value(plugin.complete(serde_json::from_value(params)?)?)?
        }
        method if method == provider::Capability::Speech.method() => {
            serde_json::to_value(plugin.synthesize(serde_json::from_value(params)?)?)?
        }
        _ => anyhow::bail!("Unknown plugin method"),
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

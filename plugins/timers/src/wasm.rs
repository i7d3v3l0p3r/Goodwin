use alloc::string::String;
use core::{
    panic::PanicInfo,
    slice,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, ensure};
use goodwin_plugin::Plugin;
use serde::Deserialize;
use serde_json::{Value, json};
use spin::Mutex;

use crate::{Timers, state::Clock};

const FRAME_LIMIT: usize = 128 * 1024;
static mut BUFFER: [u8; FRAME_LIMIT] = [0; FRAME_LIMIT];
static TIME: AtomicU64 = AtomicU64::new(0);
static PLUGIN: Mutex<Option<Timers>> = Mutex::new(None);

#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "goodwin")]
unsafe extern "C" {
    fn play_timer_signal();
}

pub fn now() -> u64 {
    TIME.load(Ordering::Relaxed)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    id: u64,
    method: String,
    params: Value,
    now_ms: u64,
}

fn dispatch(mut request: Request) -> Result<Value> {
    let mut plugin = PLUGIN
        .try_lock()
        .context("Timers call already in progress")?;
    TIME.store(request.now_ms, Ordering::Relaxed);
    if request.method == "initialize" {
        ensure!(plugin.is_none(), "Timers already initialized");
        let state = request
            .params
            .get_mut("state")
            .map(Value::take)
            .unwrap_or_default();
        let state = if state.is_null() {
            Default::default()
        } else {
            serde_json::from_value(state)?
        };
        *plugin = Some(Timers {
            clock: Clock::from_state(state),
            choices: None,
        });
    }
    let plugin = plugin.as_mut().context("Timers not initialized")?;
    let result = match request.method.as_str() {
        "initialize" => plugin.registration(),
        "http" => serde_json::to_value(plugin.http(serde_json::from_value(request.params)?)?)?,
        "tool" => plugin.tool(serde_json::from_value(request.params)?)?,
        "tick" => {
            for _ in 0..plugin.clock.finish_due()? {
                // The host queues sounds after persisting the new state.
                unsafe { play_timer_signal() };
            }
            Value::Null
        }
        _ => anyhow::bail!("Unsupported Timers method"),
    };
    let mut reply = json!({"jsonrpc":"2.0", "id":request.id, "result":result, "wakeAt":plugin.clock.next_deadline()});
    if let Some(state) = plugin.clock.take_state()? {
        reply["state"] = state;
    }
    Ok(reply)
}

#[unsafe(no_mangle)]
extern "C" fn goodwin_buffer() -> *mut u8 {
    (&raw mut BUFFER).cast()
}

#[unsafe(no_mangle)]
extern "C" fn goodwin_call(length: u32) -> i32 {
    let length = length as usize;
    if length > FRAME_LIMIT {
        return -1;
    }
    // The host runs each instance serially and grants access to this buffer for the call.
    let buffer = unsafe { slice::from_raw_parts_mut(goodwin_buffer(), FRAME_LIMIT) };
    let Ok(request) = serde_json::from_slice::<Request>(&buffer[..length]) else {
        return -1;
    };
    let id = request.id;
    let result = dispatch(request).unwrap_or_else(|_| json!({
        "jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "Timer request failed"},
    }));
    match serde_json::to_vec(&result) {
        Ok(output) if output.len() <= FRAME_LIMIT => {
            buffer[..output.len()].copy_from_slice(&output);
            output.len() as i32
        }
        _ => -1,
    }
}

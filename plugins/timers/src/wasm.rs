use core::{
    error::Error,
    fmt,
    panic::PanicInfo,
    slice,
    sync::atomic::{AtomicU64, Ordering},
};

use goodwin_plugin::rpc::{Frame, FrameError};
use serde::Deserialize;
use serde_json::{Value, json};
use spin::Mutex;

use crate::{
    Timers,
    state::{Clock, ClockError, State},
};

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
    state: Option<State>,
}

#[derive(Debug)]
enum CallError {
    Busy,
    MissingTime,
    AlreadyInitialized,
    NotInitialized,
    UnknownMethod,
    Parameters(FrameError),
    Timer(ClockError),
    Response(serde_json::Error),
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "Timers call already in progress",
            Self::MissingTime => "Missing host timestamp",
            Self::AlreadyInitialized => "Timers already initialized",
            Self::NotInitialized => "Timers not initialized",
            Self::UnknownMethod => "Unsupported Timers method",
            Self::Parameters(_) => "Read Timers request parameters",
            Self::Timer(_) => "Update timer state",
            Self::Response(_) => "Encode Timers response",
        })
    }
}

impl Error for CallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parameters(error) => Some(error),
            Self::Timer(error) => Some(error),
            Self::Response(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FrameError> for CallError {
    fn from(error: FrameError) -> Self {
        Self::Parameters(error)
    }
}

impl From<ClockError> for CallError {
    fn from(error: ClockError) -> Self {
        Self::Timer(error)
    }
}

impl From<serde_json::Error> for CallError {
    fn from(error: serde_json::Error) -> Self {
        Self::Response(error)
    }
}

fn dispatch(request: Frame<'_>) -> Result<Value, CallError> {
    let mut plugin = PLUGIN.try_lock().ok_or(CallError::Busy)?;
    TIME.store(
        request.now_ms.ok_or(CallError::MissingTime)?,
        Ordering::Relaxed,
    );
    if request.method == "initialize" {
        if plugin.is_some() {
            return Err(CallError::AlreadyInitialized);
        }
        let parameters: Request = request.params()?;
        *plugin = Some(Timers {
            clock: Clock::from_state(parameters.state.unwrap_or_default()),
            choices: None,
        });
    }
    let plugin = plugin.as_mut().ok_or(CallError::NotInitialized)?;
    let result = match request.method.as_str() {
        "initialize" => plugin.registration(),
        "http" => serde_json::to_value(plugin.http(request.params()?)?)?,
        "tool" => plugin.tool(request.params()?)?,
        "tick" => {
            for _ in 0..plugin.clock.finish_due()? {
                // The host queues sounds after persisting the new state.
                unsafe { play_timer_signal() };
            }
            Value::Null
        }
        _ => return Err(CallError::UnknownMethod),
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
    let Ok(request) = Frame::read(&buffer[..length]) else {
        return -1;
    };
    let id = request.id;
    let result = dispatch(request).unwrap_or_else(|_| json!({
        "jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "Timer request failed"},
    }));
    goodwin_plugin::json_codec::to_slice(&result, buffer).map_or(-1, |length| length as i32)
}

#[cfg(not(target_os = "espidf"))]
mod app;
#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
mod assistant;
#[cfg(target_os = "espidf")]
mod builtin_assistant;
#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
#[path = "../shared/checksum.rs"]
mod checksum;
#[cfg(not(target_os = "espidf"))]
mod config;
#[cfg(not(target_os = "espidf"))]
mod core_update;
#[cfg(not(target_os = "espidf"))]
#[path = "../shared/http_tls.rs"]
mod http_tls;
#[cfg(not(target_os = "espidf"))]
mod journal;
mod journal_format;
mod model;
#[cfg(not(target_os = "espidf"))]
mod panel;
mod panel_core;
mod password;
#[cfg(not(target_os = "espidf"))]
mod persistence;
mod platform;
#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
mod plugin_rpc;
#[cfg(all(
    feature = "wasm-plugins",
    any(target_os = "linux", target_os = "espidf")
))]
mod plugin_wasm;
#[cfg(any(not(target_os = "espidf"), feature = "wasm-plugins"))]
mod plugins;
#[cfg(not(target_os = "espidf"))]
mod providers;
#[cfg(not(target_os = "espidf"))]
mod station;
#[cfg(all(test, not(target_os = "espidf")))]
mod test_support;
mod voice_core;
#[cfg(not(target_os = "espidf"))]
mod wake;
#[cfg(any(not(target_os = "espidf"), feature = "wake-tflite"))]
#[path = "../shared/wake_catalog.rs"]
mod wake_catalog;
#[cfg(feature = "wake-tflite")]
#[path = "wake/tflite.rs"]
mod wake_tflite;
#[cfg(not(target_os = "espidf"))]
mod wav;

fn main() -> goodwin_error::Result<()> {
    platform::run()
}

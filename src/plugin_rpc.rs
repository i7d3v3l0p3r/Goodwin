use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Serialize)]
pub(crate) struct Request<'a, T> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: T,
    #[serde(rename = "nowMs", skip_serializing_if = "Option::is_none")]
    pub now: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) struct Response<T> {
    pub jsonrpc: String,
    #[cfg(not(target_os = "espidf"))]
    pub id: Option<u64>,
    #[cfg(target_os = "espidf")]
    pub id: u64,
    #[cfg(not(target_os = "espidf"))]
    pub method: Option<String>,
    #[cfg(not(target_os = "espidf"))]
    pub params: Option<Box<serde_json::value::RawValue>>,
    #[serde(default, deserialize_with = "present")]
    pub result: Option<T>,
    pub error: Option<RpcError>,
    pub wake_subscription: Option<u64>,
    #[cfg(any(
        target_os = "espidf",
        all(target_os = "linux", feature = "wasm-plugins")
    ))]
    #[serde(default, deserialize_with = "present")]
    pub state: Option<Value>,
    #[cfg(any(
        target_os = "espidf",
        all(target_os = "linux", feature = "wasm-plugins")
    ))]
    #[serde(default, deserialize_with = "present")]
    pub wake_at: Option<Option<u64>>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[cfg(target_os = "espidf")]
pub(crate) type RpcError = serde::de::IgnoredAny;

#[cfg(not(target_os = "espidf"))]
#[derive(Deserialize)]
pub(crate) struct RpcError {
    pub code: i64,
    #[serde(default)]
    pub data: Value,
}

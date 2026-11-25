use goodwin_error::Result;
use goodwin_plugin::{provider::MAX_RECORDING_BYTES, services::Request};

mod validation;
mod wamr;
pub(crate) use wamr::Guest;

pub(crate) const MODULE_LIMIT: usize = 512 * 1024;
pub(crate) const FRAME_LIMIT: usize = 128 * 1024;
pub(crate) const META_LIMIT: usize = 32 * 1024;
pub(crate) const INPUT_LIMIT: usize = MAX_RECORDING_BYTES.div_ceil(3) * 4 + FRAME_LIMIT;
pub(crate) const MAX_SIGNALS: u32 = 32;

pub(crate) trait HostServices: Send + 'static {
    fn execute(&mut self, request: Request, input: &[u8]) -> Result<Vec<u8>>;
}

#[cfg(all(test, not(target_os = "espidf")))]
pub(crate) type Reply = (serde_json::Value, u32);

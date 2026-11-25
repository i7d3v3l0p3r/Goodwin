mod indicators;

pub(crate) use indicators::Indicators;

use crate::{platform::linux::audio::yio::Capture, wake, wake_catalog::Runtime};

use super::Capabilities;

pub(crate) const PLAYBACK_DEVICE: &str = "speakers";

pub(crate) fn microphone() -> anyhow::Result<Capture> {
    Capture::from_env("127.0.0.1:9850")
}

pub(crate) fn capabilities() -> Capabilities<Runtime> {
    Capabilities {
        platform: "linux",
        device: "mini3",
        activation: &["wake_word", "manual"],
        indicators: &["logo", "clock"],
        wake_runtimes: wake::runtimes(),
        speech_streaming: true,
        panel_updates: "events",
        provider_configuration: "plugins",
        plugin_packages: true,
        wake_catalog: true,
    }
}

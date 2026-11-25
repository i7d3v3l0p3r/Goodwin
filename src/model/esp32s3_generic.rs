use esp_idf_svc::hal::{gpio::Pins, i2s::I2S0};

use crate::platform::espidf::audio::{Audio, Wiring};

use super::Capabilities;

#[cfg(feature = "wake-tflite")]
pub(crate) const WAKE_WORD: &str = "Гудвин";
#[cfg(feature = "wake-tflite")]
pub(crate) const WAKE_THRESHOLD: f32 = 0.390625;
#[cfg(feature = "wake-tflite")]
pub(crate) const WAKE_MODEL_ID: &str = "goodwin-micro-int8";
#[cfg(feature = "wake-tflite")]
pub(crate) const WAKE_MODEL_SHA: &str =
    "985a2f86cc3a7de4ec2489a1709ce70ceac210a3e30c6af858c5b8a5a385c24c";
#[cfg(feature = "wake-tflite")]
pub(crate) const WAKE_RUNTIME: &str = "livekit-v2-micro-tflite";

pub(crate) const FIRMWARE_TAG: &[u8] = if cfg!(feature = "esp-emulator") {
    b"Goodwin/esp32s3-generic/emulator/1\0"
} else {
    b"Goodwin/esp32s3-generic/production/1\0"
};

pub(crate) fn capabilities() -> Capabilities<&'static str> {
    Capabilities {
        platform: "espidf",
        device: "esp32s3-generic",
        activation: &[
            "push_to_talk",
            #[cfg(feature = "wake-tflite")]
            "wake_word",
        ],
        indicators: &[],
        wake_runtimes: &[
            #[cfg(feature = "wake-tflite")]
            WAKE_RUNTIME,
        ],
        speech_streaming: true,
        panel_updates: "poll",
        provider_configuration: "builtin",
        plugin_packages: cfg!(feature = "wasm-plugins"),
        wake_catalog: cfg!(feature = "wake-tflite"),
    }
}

pub(crate) fn audio(i2s: I2S0<'static>, pins: Pins) -> goodwin_error::Result<Audio<'static>> {
    Audio::new(
        i2s,
        Wiring {
            button: pins.gpio4.into(),
            bit_clock: pins.gpio5.into(),
            word_select: pins.gpio15.into(),
            microphone: pins.gpio7.into(),
            speaker: pins.gpio6.into(),
            mic_slot: 0,
        },
    )
}

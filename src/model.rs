#[cfg(goodwin_model = "esp32s3-generic")]
mod esp32s3_generic;
#[cfg(goodwin_model = "mini3")]
mod mini3;

#[cfg(goodwin_model = "esp32s3-generic")]
pub(crate) use esp32s3_generic::*;
#[cfg(goodwin_model = "mini3")]
pub(crate) use mini3::*;

#[derive(serde::Serialize)]
pub(crate) struct Capabilities<R: 'static> {
    pub platform: &'static str,
    pub device: &'static str,
    pub activation: &'static [&'static str],
    pub indicators: &'static [&'static str],
    pub wake_runtimes: &'static [R],
    pub wake_catalog: bool,
    pub speech_streaming: bool,
    pub panel_updates: &'static str,
    pub provider_configuration: &'static str,
    pub plugin_packages: bool,
}

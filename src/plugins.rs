pub(crate) mod connections;
#[cfg(not(target_os = "espidf"))]
mod manager;
pub(crate) mod package;
#[cfg(not(target_os = "espidf"))]
mod process;
pub(crate) mod registration;
#[cfg(not(target_os = "espidf"))]
mod registry;
pub(crate) mod settings;
pub(crate) mod store;
#[cfg(not(target_os = "espidf"))]
mod tools;

#[cfg(not(target_os = "espidf"))]
pub(crate) use connections::Update as ConnectionUpdate;
pub(crate) use connections::valid_provider_id;
#[cfg(not(target_os = "espidf"))]
pub use manager::Plugins;
#[cfg(not(target_os = "espidf"))]
pub use package::ARCHIVE_LIMIT;
#[cfg(not(target_os = "espidf"))]
pub(crate) use process::Process;
#[cfg(not(target_os = "espidf"))]
pub(crate) use process::SpeechReplies;
#[cfg(not(target_os = "espidf"))]
pub use registry::View;
#[cfg(not(target_os = "espidf"))]
pub(crate) use settings::BODY_LIMIT as SETTINGS_BODY_LIMIT;
#[cfg(not(target_os = "espidf"))]
pub use tools::VoiceTools;

#[derive(Clone, Copy)]
pub enum Failure {
    InvalidPackage,
    InvalidSettings,
    NotFound,
    #[cfg(not(target_os = "espidf"))]
    PasswordRequired,
    StartFailed,
    RolledBack,
    #[cfg(target_os = "espidf")]
    Busy,
    Storage,
    Unavailable,
}

impl Failure {
    pub(crate) fn status(self) -> u16 {
        match self {
            Self::InvalidPackage | Self::InvalidSettings => 400,
            Self::NotFound => 404,
            #[cfg(not(target_os = "espidf"))]
            Self::PasswordRequired => 403,
            Self::StartFailed | Self::RolledBack => 409,
            #[cfg(target_os = "espidf")]
            Self::Busy => 409,
            Self::Storage => 500,
            Self::Unavailable => 503,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalid_plugin_package",
            Self::InvalidSettings => "invalid_plugin_settings",
            Self::NotFound => "plugin_not_found",
            #[cfg(not(target_os = "espidf"))]
            Self::PasswordRequired => "plugin_password_required",
            Self::StartFailed => "plugin_start_failed",
            Self::RolledBack => "plugin_update_rolled_back",
            #[cfg(target_os = "espidf")]
            Self::Busy => "plugin_busy",
            Self::Storage => "plugin_storage_failed",
            Self::Unavailable => "plugin_unavailable",
        }
    }
}

mod connections;
mod manager;
mod package;
mod process;
mod registry;
mod settings;
mod store;
mod tools;

pub(crate) use connections::Update as ConnectionUpdate;
pub(crate) use connections::valid_provider_id;
pub use manager::Plugins;
pub use package::ARCHIVE_LIMIT;
pub(crate) use process::Process;
pub(crate) use process::SpeechReplies;
pub use registry::View;
pub(crate) use settings::BODY_LIMIT as SETTINGS_BODY_LIMIT;
pub use tools::VoiceTools;

#[derive(Clone, Copy)]
pub enum Failure {
    InvalidPackage,
    InvalidSettings,
    NotFound,
    PasswordRequired,
    StartFailed,
    RolledBack,
    Storage,
    Unavailable,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalid_plugin_package",
            Self::InvalidSettings => "invalid_plugin_settings",
            Self::NotFound => "plugin_not_found",
            Self::PasswordRequired => "plugin_password_required",
            Self::StartFailed => "plugin_start_failed",
            Self::RolledBack => "plugin_update_rolled_back",
            Self::Storage => "plugin_storage_failed",
            Self::Unavailable => "plugin_unavailable",
        }
    }
}

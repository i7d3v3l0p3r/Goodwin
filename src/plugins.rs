mod manager;
mod package;
mod process;
mod registry;
mod store;
mod tools;

pub use manager::Plugins;
pub use package::ARCHIVE_LIMIT;
pub use registry::View;
pub use tools::VoiceTools;

#[derive(Clone, Copy)]
pub enum Failure {
    InvalidPackage,
    NotFound,
    StartFailed,
    RolledBack,
    Storage,
    Unavailable,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalid_plugin_package",
            Self::NotFound => "plugin_not_found",
            Self::StartFailed => "plugin_start_failed",
            Self::RolledBack => "plugin_update_rolled_back",
            Self::Storage => "plugin_storage_failed",
            Self::Unavailable => "plugin_unavailable",
        }
    }
}

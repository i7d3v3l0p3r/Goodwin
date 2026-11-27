#[cfg(any(target_os = "linux", test))]
mod profile;
#[cfg(target_os = "linux")]
mod service;
#[cfg(any(target_os = "linux", test))]
#[path = "../profile-types.rs"]
mod types;

fn main() -> goodwin_error::Result<()> {
    #[cfg(target_os = "linux")]
    return service::run();
    #[cfg(not(target_os = "linux"))]
    goodwin_error::bail!("The Goodwin VPN plugin requires Linux")
}

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
pub(crate) struct Status {
    pub(crate) version: &'static str,
    pub(crate) supported: bool,
    job: Option<Job>,
}

fn version_parts(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        let part = parts.next()?;
        if !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *number = part.parse().ok()?;
    }
    parts.next().is_none().then_some(numbers)
}

pub(crate) fn newer(version: &str, current: &str) -> bool {
    matches!((version_parts(version), version_parts(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
mod station;

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub(crate) use station::run_worker;

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
use station::Job;

#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
type Job = ();

pub(crate) fn status(dir: &Path) -> Result<Status> {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    if station::supported(dir) {
        return Ok(Status {
            version: VERSION,
            supported: true,
            job: station::read_job()?,
        });
    }
    let _ = dir;
    Ok(Status {
        version: VERSION,
        supported: false,
        job: None,
    })
}

pub(crate) fn latest() -> Result<String> {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        station::latest()
    }

    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    anyhow::bail!("Core updates are unavailable on this platform")
}

pub(crate) async fn start(version: String) -> Result<bool> {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        station::start(version).await
    }

    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    {
        let _ = version;
        anyhow::bail!("Core updates are unavailable on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn only_newer_stable_versions_can_be_installed() {
        assert!(newer("0.10.0", "0.9.12"));
        for version in [
            "0.9.12",
            "0.8.20",
            "0.10.0-rc.1",
            "00.10.0",
            "1.0.+1",
            "../1.0.0",
        ] {
            assert!(!newer(version, "0.9.12"), "{version}");
        }
    }
}

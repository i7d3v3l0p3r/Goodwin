use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rustix::fs::statvfs;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::journal::{TAIL_BYTES, clip_tail};

use super::{Resources, Usage};

static LOG_READ: Mutex<()> = Mutex::const_new(());

pub(super) async fn resources(dir: PathBuf) -> Resources {
    let first = tokio::task::spawn_blocking(cpu_sample).await.ok().flatten();
    sleep(Duration::from_millis(250)).await;
    tokio::task::spawn_blocking(move || {
        let cpu = first
            .zip(cpu_sample())
            .and_then(|(first, second)| second.usage_since(first));
        Resources {
            supported: true,
            cpu_percent: cpu.map(|(system, _)| system),
            goodwin_cpu_percent: cpu.and_then(|(_, process)| process),
            memory: fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|info| memory_usage(&info)),
            storage: storage_usage(&dir),
            uptime_seconds: fs::read_to_string("/proc/uptime")
                .ok()
                .and_then(|info| info.split('.').next()?.parse().ok()),
            goodwin_rss_bytes: fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|status| bytes_from_kib(&status, "VmRSS:")),
        }
    })
    .await
    .unwrap_or(Resources {
        supported: true,
        ..Resources::default()
    })
}

struct CpuSample {
    total: u64,
    idle: u64,
    process: Option<u64>,
}

impl CpuSample {
    fn parse(system: &str, process: Option<&str>) -> Option<Self> {
        let mut fields = system.lines().next()?.split_whitespace();
        if fields.next()? != "cpu" {
            return None;
        }
        // Guest counters are already included in user and nice.
        let ticks: Vec<u64> = fields
            .take(8)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let idle = ticks.get(3)? + ticks.get(4).copied().unwrap_or_default();
        let process = process.and_then(|stat| {
            // The process name is in parentheses and can contain spaces or parentheses.
            let (_, fields) = stat.rsplit_once(')')?;
            let mut fields = fields.split_whitespace().skip(11);
            let user = fields.next()?.parse::<u64>().ok()?;
            user.checked_add(fields.next()?.parse().ok()?)
        });
        Some(Self {
            total: ticks.iter().sum(),
            idle,
            process,
        })
    }

    fn usage_since(self, first: Self) -> Option<(f64, Option<f64>)> {
        let total = self.total.checked_sub(first.total)?;
        if total == 0 {
            return None;
        }
        let percent = |ticks| 100.0 * ticks as f64 / total as f64;
        let idle = self.idle.saturating_sub(first.idle);
        let process = self
            .process
            .zip(first.process)
            .and_then(|(second, first)| second.checked_sub(first))
            .map(|ticks| percent(ticks.min(total)));
        Some((percent(total.saturating_sub(idle)), process))
    }
}

fn cpu_sample() -> Option<CpuSample> {
    let system = fs::read_to_string("/proc/stat").ok()?;
    let process = fs::read_to_string("/proc/self/stat").ok();
    CpuSample::parse(&system, process.as_deref())
}

fn bytes_from_kib(info: &str, key: &str) -> Option<u64> {
    let mut fields = info
        .lines()
        .find_map(|line| line.strip_prefix(key))?
        .split_whitespace();
    let value = fields.next()?.parse::<u64>().ok()?;
    (fields.next()? == "kB").then(|| value.saturating_mul(1024))
}

fn memory_usage(info: &str) -> Option<Usage> {
    let total_bytes = bytes_from_kib(info, "MemTotal:")?;
    let available = bytes_from_kib(info, "MemAvailable:")?;
    Some(Usage {
        used_bytes: total_bytes.saturating_sub(available),
        total_bytes,
    })
}

fn storage_usage(dir: &Path) -> Option<Usage> {
    let stats = statvfs(dir).ok()?;
    Some(Usage {
        used_bytes: stats
            .f_blocks
            .saturating_sub(stats.f_bfree)
            .saturating_mul(stats.f_frsize),
        total_bytes: stats.f_blocks.saturating_mul(stats.f_frsize),
    })
}

pub(super) async fn read_kernel_log() -> Option<String> {
    let _guard = LOG_READ.lock().await;
    timeout(Duration::from_secs(3), kernel_log())
        .await
        .ok()
        .flatten()
}

async fn kernel_log() -> Option<String> {
    let mut child = Command::new("dmesg")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = stdout.read(&mut buffer).await.ok()?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > 2 * TAIL_BYTES {
            bytes.drain(..bytes.len() - TAIL_BYTES);
            truncated = true;
        }
    }
    child
        .wait()
        .await
        .ok()?
        .success()
        .then(|| clip_tail(&bytes, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_uses_deltas_without_counting_guests_twice() {
        let before = CpuSample::parse(
            "cpu 30 0 20 100 10 0 0 0 5 0",
            Some("1 (good win)) S 0 0 0 0 0 0 0 0 0 0 1 1"),
        )
        .unwrap();
        let after = CpuSample::parse(
            "cpu 50 0 30 160 20 0 0 0 15 0",
            Some("1 (good win)) S 0 0 0 0 0 0 0 0 0 0 8 4"),
        )
        .unwrap();
        assert_eq!(after.usage_since(before), Some((30.0, Some(10.0))));
    }

    #[test]
    fn memory_usage_subtracts_available_memory() {
        let usage =
            memory_usage("MemTotal:  1024 kB\nMemFree: 128 kB\nMemAvailable: 768 kB").unwrap();
        assert_eq!(usage.used_bytes, 256 * 1024);
        assert_eq!(usage.total_bytes, 1024 * 1024);
    }
}

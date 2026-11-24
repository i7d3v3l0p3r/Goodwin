use std::{
    env,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Seek},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{VERSION, newer, version_parts};

const BIN: &str = "/data/opt/bin/goodwin";
const DATA: &str = "/data/opt/etc/goodwin";
const INIT: &str = "/data/init.d/S96goodwin";
const PID_FILE: &str = "/tmp/goodwin.pid";
const OPT: &str = "/data/opt";
const LOCK: &str = "/data/opt/.goodwin-core-update.lock";
const JOB: &str = "/data/opt/.goodwin-core-update.json";
const REPO: &str = "i7d3v3l0p3r/Goodwin";
const ARCHIVE: &str = "goodwin-mini3-aarch64.tar.gz";
const PACKAGE_LIMIT: u64 = 128 * 1024 * 1024;
const METADATA_LIMIT: u64 = 64 * 1024;

#[derive(Deserialize, Serialize)]
pub(super) struct Job {
    version: String,
    phase: Phase,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Downloading,
    Installing,
    Completed,
    DownloadFailed,
    InstallFailed,
    RolledBack,
    Interrupted,
}

fn installed_executable(path: &str) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub(super) fn supported(dir: &Path) -> bool {
    installed_executable(BIN)
        && installed_executable(INIT)
        && fs::canonicalize(dir)
            .ok()
            .zip(fs::canonicalize(DATA).ok())
            .is_some_and(|(a, b)| a == b)
        && env::current_exe()
            .ok()
            .zip(fs::canonicalize(BIN).ok())
            .is_some_and(|(a, b)| a == b)
        && fs::read_to_string(PID_FILE)
            .is_ok_and(|pid| pid.trim().parse() == Ok(std::process::id()))
}

fn lock_file() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(LOCK)
}

pub(super) fn read_job() -> Result<Option<Job>> {
    let lock = lock_file()?;
    let running = match lock.try_lock_shared() {
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(error) => return Err(error.into()),
    };
    let mut job: Job = match File::open(JOB) {
        Ok(file) => serde_json::from_reader(file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !running && matches!(job.phase, Phase::Downloading | Phase::Installing) {
        job.phase = Phase::Interrupted;
    }
    Ok(Some(job))
}

impl Job {
    fn save(&mut self, phase: Phase) -> Result<()> {
        self.phase = phase;
        let mut file = tempfile::NamedTempFile::new_in(OPT)?;
        serde_json::to_writer(&mut file, self)?;
        file.persist(JOB)?;
        Ok(())
    }
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .user_agent(concat!("Goodwin/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()?)
}

pub(super) fn latest() -> Result<String> {
    latest_version(&client()?)
}

fn latest_version(client: &Client) -> Result<String> {
    let url = client
        .head(format!("https://github.com/{REPO}/releases/latest"))
        .timeout(Duration::from_secs(15))
        .send()?
        .error_for_status()?
        .url()
        .clone();
    let version = url
        .path()
        .strip_prefix(&format!("/{REPO}/releases/tag/v"))
        .filter(|version| url.host_str() == Some("github.com") && version_parts(version).is_some())
        .context("Could not determine the latest stable Goodwin release")?;
    Ok(version.to_owned())
}

pub(super) async fn start(version: String) -> Result<bool> {
    let mut child = tokio::process::Command::new(BIN)
        .args(["core-update-worker", &version])
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().context("Missing updater output")?;
    let mut lines = BufReader::new(stdout).lines();
    let result = tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    match result??.as_deref() {
        Some("started") => Ok(true),
        Some("busy") => Ok(false),
        _ => bail!("Goodwin updater exited before starting"),
    }
}

pub(crate) fn run_worker(version: &str) -> Result<()> {
    ensure!(newer(version, VERSION), "Invalid update version");
    let lock = lock_file()?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            println!("busy");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    let mut job = Job {
        version: version.into(),
        phase: Phase::Downloading,
    };
    job.save(Phase::Downloading)?;
    println!("started");

    let result = (|| -> Result<Phase> {
        let upload = tempfile::Builder::new()
            .prefix(".goodwin-upload.")
            .tempdir_in(OPT)?;
        let client = client()?;
        ensure!(
            latest_version(&client)? == version,
            "The latest release changed"
        );
        prepare(&client, version, upload.path())?;
        let script = upload.path().join("install-station.sh");
        fs::write(
            &script,
            include_str!("../../installer/install-station.sh").replace("\r\n", "\n"),
        )?;
        job.save(Phase::Installing)?;
        let output = Command::new("sh")
            .arg(script)
            .arg(upload.path())
            .args(["core", VERSION])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprint!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        if stdout
            .lines()
            .any(|line| line == "GOODWIN_CORE_ROLLED_BACK")
        {
            return Ok(Phase::RolledBack);
        }
        ensure!(output.status.success(), "Station installation failed");
        Ok(Phase::Completed)
    })();
    let phase = match &result {
        Ok(phase) => *phase,
        Err(_) if matches!(job.phase, Phase::Installing) => Phase::InstallFailed,
        Err(_) => Phase::DownloadFailed,
    };
    job.save(phase)?;
    result.map(|_| ())
}

fn prepare(client: &Client, version: &str, dir: &Path) -> Result<()> {
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let mut manifest = String::new();
    client
        .get(format!("{base}/SHA256SUMS"))
        .send()?
        .error_for_status()?
        .take(METADATA_LIMIT + 1)
        .read_to_string(&mut manifest)?;
    ensure!(
        manifest.len() as u64 <= METADATA_LIMIT,
        "Release checksum list is too large"
    );
    let mut hashes = manifest.lines().filter_map(|line| {
        let (hash, name) = line.split_once(' ')?;
        (name.strip_prefix([' ', '*'])? == ARCHIVE).then_some(hash)
    });
    let hash = hashes
        .next()
        .context("Missing checksum for the Goodwin package")?;
    ensure!(
        hashes.next().is_none()
            && hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid checksum for the Goodwin package"
    );

    let mut archive = tempfile::tempfile_in(dir)?;
    let response = client
        .get(format!("{base}/{ARCHIVE}"))
        .send()?
        .error_for_status()?;
    let size = io::copy(&mut response.take(PACKAGE_LIMIT + 1), &mut archive)?;
    ensure!(size <= PACKAGE_LIMIT, "Goodwin package is too large");
    archive.rewind()?;
    let mut digest = Sha256::new();
    io::copy(&mut archive, &mut digest)?;
    ensure!(
        hash.eq_ignore_ascii_case(&format!("{:x}", digest.finalize())),
        "Goodwin package checksum mismatch"
    );
    archive.rewind()?;

    let binary = dir.join("goodwin");
    let mut tar = tar::Archive::new(GzDecoder::new(archive).take(PACKAGE_LIMIT + 1));
    let mut found = false;
    for entry in tar.entries()?.raw(true) {
        let mut entry = entry?;
        if entry.path_bytes() == b"goodwin-mini3/goodwin".as_slice() {
            ensure!(
                !found && entry.header().entry_type().is_file(),
                "Invalid executable entry"
            );
            let mut header = [0; 20];
            entry.read_exact(&mut header)?;
            ensure!(
                &header[..6] == b"\x7fELF\x02\x01" && header[18..20] == [0xb7, 0],
                "Expected an AArch64 ELF executable"
            );
            let mut file = File::create(&binary)?;
            use std::io::Write;
            file.write_all(&header)?;
            let rest = io::copy(&mut entry, &mut file)?;
            ensure!(
                rest + header.len() as u64 == entry.size(),
                "Truncated Goodwin executable"
            );
            file.set_permissions(fs::Permissions::from_mode(0o755))?;
            file.sync_all()?;
            found = true;
        }
    }
    ensure!(found, "Goodwin executable is missing");
    let output = Command::new(binary).arg("--version").output()?;
    ensure!(
        output.status.success() && std::str::from_utf8(&output.stdout)?.trim() == version,
        "Goodwin executable does not match the release"
    );
    Ok(())
}

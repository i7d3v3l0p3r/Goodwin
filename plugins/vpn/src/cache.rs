use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use flate2::read::MultiGzDecoder;
use goodwin_error::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Catalog {
    arch: String,
    base_url: String,
    workers: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct Asset {
    size: u64,
    gzip_size: u64,
    sha256: String,
}

fn digest(reader: &mut impl Read, sink: &mut impl Write, limit: u64) -> Result<(u64, String)> {
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 16384];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(total <= limit, "Engine exceeds expected size");
        hash.update(&buffer[..n]);
        sink.write_all(&buffer[..n])?;
    }
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(hex, "{byte:02x}")?;
    }
    Ok((total, hex))
}

impl Asset {
    fn matches(&self, file: &Path) -> Result<bool> {
        let mut file = match File::open(file) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if file.metadata()?.len() != self.size {
            return Ok(false);
        }
        let (size, hash) = digest(&mut file, &mut io::sink(), self.size)?;
        Ok(size == self.size && hash == self.sha256)
    }

    fn fetch(&self, url: &str, stage: &Path) -> Result<()> {
        let config = ureq::Agent::config_builder()
            .https_only(url.starts_with("https://"))
            .timeout_global(Some(Duration::from_secs(90)))
            .build();
        let agent: ureq::Agent = config.into();
        let mut response = agent.get(url).call()?;
        ensure!(response.status().as_u16() == 200, "Engine download failed");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(stage)?;
        self.unpack(response.body_mut().as_reader(), &mut file)?;
        file.set_permissions(fs::Permissions::from_mode(0o700))?;
        file.sync_all()?;
        Ok(())
    }

    fn unpack(&self, reader: impl Read, dest: &mut impl Write) -> Result<()> {
        let mut decoder = MultiGzDecoder::new(reader.take(self.gzip_size + 1));
        let (size, hash) = digest(&mut decoder, dest, self.size)?;
        ensure!(
            size == self.size && hash == self.sha256 && decoder.into_inner().limit() == 1,
            "Invalid engine download"
        );
        Ok(())
    }
}

fn cache_dir() -> Result<PathBuf> {
    Ok(goodwin_plugin::data_dir()?.join("engines"))
}

fn lock_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.lock"))
}
fn partial_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.part"))
}

pub fn discard_partial(name: &str) {
    if let Ok(dir) = cache_dir()
        && let Ok(lock) = File::open(lock_path(&dir, name))
        && lock.try_lock().is_ok()
    {
        let _ = fs::remove_file(partial_path(&dir, name));
    }
}

/// Run in the candidate process. exec releases the downloader's heap, TLS sessions
/// and file lock, preserving the controller's PID tracking and private stdio.
pub fn launch(name: &str, socket: &Path) -> Result<()> {
    ensure!(
        matches!(name, "vless-stream" | "vless-http2" | "vless-xtls"),
        "Unknown engine"
    );
    let catalog: Catalog = serde_json::from_reader(
        File::open(std::env::current_exe()?.with_file_name("engines.json"))?.take(8192),
    )?;
    ensure!(
        catalog.arch == std::env::consts::ARCH,
        "Engine architecture mismatch"
    );
    let asset = catalog
        .workers
        .get(name)
        .context("Missing engine in catalog")?;
    ensure!(
        (1..=32 * 1024 * 1024).contains(&asset.size)
            && (1..=32 * 1024 * 1024).contains(&asset.gzip_size)
            && asset.sha256.len() == 64
            && asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid engine metadata"
    );
    let dir = cache_dir()?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path(&dir, name))?;
    lock.lock()?;
    let executable = dir.join(name);
    let stage = partial_path(&dir, name);
    if let Err(e) = fs::remove_file(&stage)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e.into());
    }
    if !asset.matches(&executable)? {
        let url = format!("{}/{name}-linux-{}.gz", catalog.base_url, catalog.arch);
        let installed = asset.fetch(&url, &stage).and_then(|()| {
            fs::rename(&stage, &executable)?;
            File::open(&dir)?.sync_all()?;
            Ok(())
        });
        if installed.is_err() {
            let _ = fs::remove_file(&stage);
        }
        installed?;
    } else {
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    }
    // Only request the credentials from the controller once the executable is verified.
    let mut stdout = io::stdout().lock();
    stdout.write_all(&[0])?;
    stdout.flush()?;
    drop(stdout);
    Err(Command::new(executable).arg(socket).exec().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    #[test]
    fn installation_requires_the_pinned_digest_and_both_lengths() {
        let mut zip = GzEncoder::new(Vec::new(), Compression::fast());
        zip.write_all(b"abc").unwrap();
        let packed = zip.finish().unwrap();
        let mut asset = Asset {
            size: 3,
            gzip_size: packed.len() as u64,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        let mut decoded = Vec::new();
        asset.unpack(&packed[..], &mut decoded).unwrap();
        assert_eq!(decoded, b"abc");
        assert!(
            asset
                .unpack(&packed[..packed.len() - 1], &mut io::sink())
                .is_err()
        );
        asset.gzip_size += 1;
        assert!(asset.unpack(&packed[..], &mut io::sink()).is_err());
        asset.gzip_size -= 1;
        asset.size -= 1;
        assert!(asset.unpack(&packed[..], &mut io::sink()).is_err());
        asset.size += 1;
        asset.sha256.replace_range(..1, "0");
        assert!(asset.unpack(&packed[..], &mut io::sink()).is_err());
    }
}

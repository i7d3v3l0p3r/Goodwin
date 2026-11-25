use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use url::Url;

use super::settings::{self, Field};

#[cfg(not(target_os = "espidf"))]
pub const ARCHIVE_LIMIT: usize = 16 * 1024 * 1024;
#[cfg(not(target_os = "espidf"))]
const EXPANDED_LIMIT: u64 = 48 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 32 * 1024;
pub const API_VERSION: u32 = 1;
pub const UI_KIT_VERSION: u32 = 1;

#[derive(Clone, Deserialize, Serialize)]
pub struct Label {
    pub en: String,
    pub ru: String,
}

impl Label {
    pub fn valid(&self) -> bool {
        [&self.en, &self.ru].into_iter().all(|text| {
            !text.trim().is_empty() && text.len() <= 160 && !text.chars().any(char::is_control)
        })
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub title: Label,
    api_version: u32,
    target: String,
    pub executable: String,
    pub(crate) wasm_state_file: Option<String>,
    pub(crate) wasm_http: Option<WasmHttp>,
    #[serde(default)]
    pub(crate) provider_connections: bool,
    #[serde(default)]
    #[cfg_attr(
        not(all(feature = "wasm-plugins", target_os = "linux")),
        allow(dead_code)
    )]
    pub(crate) wasm_storage: bool,
    pub web: Option<Web>,
    #[serde(default)]
    pub(crate) settings: Vec<Field>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WasmHttp {
    pub(crate) base_url: Option<String>,
    pub(crate) base_url_setting: Option<String>,
    pub(crate) key_setting: String,
    #[serde(default)]
    #[cfg_attr(
        not(all(feature = "wasm-plugins", target_os = "linux")),
        allow(dead_code)
    )]
    pub(crate) require_key: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Web {
    ui_kit_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    solid_runtime_version: Option<u32>,
    pub entry: String,
    #[serde(default)]
    pub styles: Vec<String>,
}

pub fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 28
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !value.ends_with('-')
}

pub fn path(value: &str) -> bool {
    value.len() <= 180
        && value.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
                && !matches!(
                    part.split('.').next().unwrap_or_default(),
                    "con"
                        | "prn"
                        | "aux"
                        | "nul"
                        | "com1"
                        | "com2"
                        | "com3"
                        | "com4"
                        | "com5"
                        | "com6"
                        | "com7"
                        | "com8"
                        | "com9"
                        | "lpt1"
                        | "lpt2"
                        | "lpt3"
                        | "lpt4"
                        | "lpt5"
                        | "lpt6"
                        | "lpt7"
                        | "lpt8"
                        | "lpt9"
                )
        })
}

impl Manifest {
    pub(crate) fn is_wasm(&self) -> bool {
        self.target == "wasm32"
    }

    pub fn read(root: &Path) -> Result<Self> {
        let file = fs::File::open(root.join("manifest.json")).context("Missing plugin manifest")?;
        ensure!(
            file.metadata()?.len() <= MANIFEST_LIMIT,
            "Plugin manifest is too large"
        );
        let manifest: Self = serde_json::from_reader(file.take(MANIFEST_LIMIT))?;
        ensure!(
            name(&manifest.id) && manifest.title.valid(),
            "Invalid plugin identity"
        );
        ensure!(
            !manifest.version.is_empty()
                && manifest.version.len() <= 48
                && manifest
                    .version
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".+-".contains(&c)),
            "Invalid plugin version"
        );
        ensure!(
            manifest.api_version == API_VERSION,
            "Unsupported plugin API version"
        );
        settings::validate(&manifest.settings)?;
        ensure!(
            !manifest.provider_connections || (manifest.is_wasm() && manifest.wasm_http.is_some()),
            "Provider connections require WASM HTTP access"
        );
        if let Some(http) = &manifest.wasm_http {
            ensure!(
                http.base_url.is_some() != http.base_url_setting.is_some()
                    && settings::wasm_http_fields(
                        &manifest.settings,
                        http.base_url_setting.as_deref(),
                        &http.key_setting,
                    ),
                "Invalid WASM HTTP configuration"
            );
            if let Some(base) = &http.base_url {
                let url = Url::parse(base)?;
                ensure!(
                    base.len() <= 2048 && url.scheme() == "https" && settings::valid_base_url(base),
                    "Invalid fixed WASM HTTP URL"
                );
            }
        }
        let native_target = !cfg!(target_os = "espidf")
            && manifest.target == format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        ensure!(
            manifest.is_wasm() || native_target,
            "Plugin target {} is unsupported on this build",
            manifest.target
        );
        ensure!(
            manifest.executable.starts_with("bin/")
                && path(&manifest.executable)
                && (!manifest.is_wasm() || manifest.executable.ends_with(".wasm"))
                && root.join(&manifest.executable).is_file(),
            "Missing plugin executable"
        );
        if let Some(file) = &manifest.wasm_state_file {
            ensure!(
                manifest.is_wasm() && path(file) && !file.contains('/') && file.ends_with(".json"),
                "Invalid WASM state filename"
            );
        }
        if let Some(web) = &manifest.web {
            ensure!(
                web.ui_kit_version == UI_KIT_VERSION,
                "Unsupported UI kit version"
            );
            ensure!(
                matches!(web.solid_runtime_version, None | Some(1)),
                "Unsupported Solid runtime version"
            );
            ensure!(
                web.entry.ends_with(".mjs") && web_file(root, &web.entry).is_some(),
                "Missing web entry module"
            );
            ensure!(
                web.styles.len() <= 8
                    && web
                        .styles
                        .iter()
                        .all(|style| style.ends_with(".css") && web_file(root, style).is_some()),
                "Invalid plugin styles"
            );
        }
        Ok(manifest)
    }

    pub fn prepare_executable(&self, _root: &Path) -> Result<()> {
        #[cfg(all(unix, not(target_os = "espidf")))]
        {
            use std::os::unix::fs::PermissionsExt;
            if !self.is_wasm() {
                fs::set_permissions(
                    _root.join(&self.executable),
                    fs::Permissions::from_mode(0o700),
                )?;
            }
        }
        Ok(())
    }
}

pub fn web_file(root: &Path, relative: &str) -> Option<PathBuf> {
    if !relative.starts_with("ui/") || !path(relative) {
        return None;
    }
    // The ESP plugin volume has no links. FAT VFS does not implement realpath.
    #[cfg(target_os = "espidf")]
    {
        let file = root.join(relative);
        file.is_file().then_some(file)
    }
    #[cfg(not(target_os = "espidf"))]
    {
        let root = root.canonicalize().ok()?;
        let file = root.join(relative).canonicalize().ok()?;
        (file.starts_with(root.join("ui")) && file.is_file()).then_some(file)
    }
}

pub(crate) fn asset_mime(path: &str) -> Option<&'static str> {
    Some(match path.rsplit_once('.')?.1 {
        "mjs" | "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        _ => return None,
    })
}

#[cfg(not(target_os = "espidf"))]
pub fn extract(bytes: &[u8], root: &Path) -> Result<Manifest> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes).take(EXPANDED_LIMIT + 1024 * 1024));
    let mut files = ArchiveFiles::new(root, EXPANDED_LIMIT)?;
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        let relative = entry.path_bytes().into_owned();
        let relative = std::str::from_utf8(&relative).context("Invalid plugin path encoding")?;
        ensure!(
            entry.header().entry_type().is_file(),
            "Archive entries must be regular files"
        );
        files.write(relative, entry.size(), &mut entry)?;
    }
    // Read the gzip trailer too, so a damaged upload cannot be accepted after the tar end marker.
    let mut stream = archive.into_inner();
    io::copy(&mut stream, &mut io::sink())?;
    ensure!(stream.limit() > 0, "Expanded plugin archive is too large");
    files.finish()
}

#[cfg(target_os = "espidf")]
pub(crate) fn extract_reader(reader: impl Read, root: &Path, limit: u64) -> Result<Manifest> {
    use std::io::BufReader;

    const BLOCK: usize = 512;
    // The package builder writes POSIX ustar with regular files and no extension entries.
    let mut stream = BufReader::new(GzDecoder::new(reader).take(limit + 1024 * 1024));
    let mut files = ArchiveFiles::new(root, limit)?;
    let mut header = [0; BLOCK];
    loop {
        stream.read_exact(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
            stream.read_exact(&mut header)?;
            ensure!(
                header.iter().all(|byte| *byte == 0),
                "Invalid tar terminator"
            );
            io::copy(&mut stream, &mut io::sink())?;
            ensure!(
                stream.get_ref().limit() > 0,
                "Expanded plugin archive is too large"
            );
            return files.finish();
        }
        ensure!(
            &header[257..263] == b"ustar\0" && matches!(header[156], b'0' | 0),
            "Expected a regular ustar file"
        );
        let checksum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, byte)| u64::from(if (148..156).contains(&i) { b' ' } else { *byte }))
            .sum();
        ensure!(
            checksum == tar_number(&header[148..156])?,
            "Invalid tar header checksum"
        );
        let name = tar_field(&header[..100])?;
        let prefix = tar_field(&header[345..500])?;
        let name = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let size = tar_number(&header[124..136])?;
        files.write(&name, size, (&mut stream).take(size))?;
        let padding = ((BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64) as usize;
        stream.read_exact(&mut header[..padding])?;
    }
}

#[cfg(target_os = "espidf")]
fn tar_field(bytes: &[u8]) -> Result<&str> {
    Ok(std::str::from_utf8(
        bytes.split(|byte| *byte == 0).next().unwrap_or_default(),
    )?)
}

#[cfg(target_os = "espidf")]
fn tar_number(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_str_radix(tar_field(bytes)?.trim_ascii(), 8)?)
}

struct ArchiveFiles<'a> {
    root: &'a Path,
    remaining: u64,
    seen: HashSet<String>,
}

impl<'a> ArchiveFiles<'a> {
    fn new(root: &'a Path, remaining: u64) -> Result<Self> {
        fs::create_dir(root)?;
        Ok(Self {
            root,
            remaining,
            seen: HashSet::new(),
        })
    }

    fn write(&mut self, relative: &str, size: u64, mut entry: impl Read) -> Result<()> {
        ensure!(path(relative), "Invalid plugin archive path");
        ensure!(
            relative == "manifest.json"
                || relative.starts_with("bin/")
                || relative.starts_with("ui/"),
            "Unexpected plugin package path"
        );
        ensure!(
            self.seen.len() < 256 && self.seen.insert(relative.to_owned()),
            "Too many or repeated archive entries"
        );
        self.remaining = self
            .remaining
            .checked_sub(size)
            .context("Expanded plugin package is too large")?;
        let output = self.root.join(relative);
        fs::create_dir_all(output.parent().expect("relative archive path"))?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        ensure!(
            io::copy(&mut entry, &mut file)? == size,
            "Incomplete archive entry"
        );
        file.sync_all()?;
        Ok(())
    }

    fn finish(self) -> Result<Manifest> {
        let manifest = Manifest::read(self.root)?;
        manifest.prepare_executable(self.root)?;
        Ok(manifest)
    }
}

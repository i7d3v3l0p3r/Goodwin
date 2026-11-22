use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};

pub const ARCHIVE_LIMIT: usize = 16 * 1024 * 1024;
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
    pub web: Option<Web>,
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
        let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        ensure!(
            manifest.target == target,
            "Plugin requires {} but this host is {target}",
            manifest.target
        );
        ensure!(
            manifest.executable.starts_with("bin/")
                && path(&manifest.executable)
                && root.join(&manifest.executable).is_file(),
            "Missing plugin executable"
        );
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                _root.join(&self.executable),
                fs::Permissions::from_mode(0o700),
            )?;
        }
        Ok(())
    }
}

pub fn web_file(root: &Path, relative: &str) -> Option<PathBuf> {
    if !relative.starts_with("ui/") || !path(relative) {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let file = root.join(relative).canonicalize().ok()?;
    (file.starts_with(root.join("ui")) && file.is_file()).then_some(file)
}

pub fn extract(bytes: &[u8], root: &Path) -> Result<Manifest> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes).take(EXPANDED_LIMIT + 1024 * 1024));
    fs::create_dir(root)?;
    let mut seen = HashSet::new();
    let mut total = 0;
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        let relative = entry.path_bytes();
        let relative = std::str::from_utf8(&relative).context("Invalid plugin path encoding")?;
        ensure!(
            entry.header().entry_type().is_file() && path(relative),
            "Archive entries must be regular files with portable relative paths"
        );
        ensure!(
            relative == "manifest.json"
                || relative.starts_with("bin/")
                || relative.starts_with("ui/"),
            "Unexpected plugin package path"
        );
        ensure!(
            seen.len() < 256 && seen.insert(relative.to_owned()),
            "Too many or repeated archive entries"
        );
        total += entry.size().min(EXPANDED_LIMIT + 1);
        ensure!(
            total <= EXPANDED_LIMIT,
            "Expanded plugin package is too large"
        );
        let output = root.join(relative);
        fs::create_dir_all(output.parent().expect("relative archive path"))?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        ensure!(
            io::copy(&mut entry, &mut file)? == entry.size(),
            "Incomplete archive entry"
        );
        file.sync_all()?;
    }
    // Read the gzip trailer too, so a damaged upload cannot be accepted after the tar end marker.
    let mut stream = archive.into_inner();
    io::copy(&mut stream, &mut io::sink())?;
    ensure!(stream.limit() > 0, "Expanded plugin archive is too large");
    let manifest = Manifest::read(root)?;
    manifest.prepare_executable(root)?;
    Ok(manifest)
}

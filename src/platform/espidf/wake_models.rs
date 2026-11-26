use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use esp_idf_svc::{
    handle::RawHandle, http::Method, sys::esp_http_client_is_complete_data_received,
};
use goodwin_error::{Context, Result, ensure};
use url::Url;

use crate::{
    model,
    wake_catalog::{CATALOG_LIMIT, Catalog, RELEASES, Runtime, Selection},
    wake_tflite::TfliteModel,
};

use super::{http_client, storage::MOUNT};

pub(super) const RUNTIMES: &[Runtime] = &[Runtime::MicroTflite];
const MODEL_LIMIT: u64 = 512 * 1024;
const REQUEST_BUFFER: usize = 2048;

pub(super) struct Candidate {
    pub selection: Selection,
    bytes: Option<Vec<u8>>,
}

impl Candidate {
    pub(super) fn builtin() -> Self {
        Self {
            selection: Selection {
                id: model::WAKE_MODEL_ID.into(),
                wake_word: model::WAKE_WORD.into(),
                tier: "micro".into(),
                sha256: model::WAKE_MODEL_SHA.into(),
                runtime: Runtime::MicroTflite,
                score_threshold: Some(model::WAKE_THRESHOLD),
            },
            bytes: None,
        }
    }

    pub(super) fn load_saved(selection: Selection) -> Result<Self> {
        selection.validate()?;
        ensure!(
            selection.runtime == Runtime::MicroTflite,
            "Unsupported saved wake runtime"
        );
        let bytes = if selection.sha256 == model::WAKE_MODEL_SHA {
            None
        } else {
            let path = asset_path(&selection);
            let file = File::open(path)?;
            ensure!(
                file.metadata()?.len() <= MODEL_LIMIT,
                "Wake model exceeds the size limit"
            );
            let mut bytes = Vec::new();
            file.take(MODEL_LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MODEL_LIMIT,
                "Wake model exceeds the size limit"
            );
            selection.verify_sha256(&bytes)?;
            Some(bytes)
        };
        Ok(Self { selection, bytes })
    }

    pub(super) fn load(self, builtin: &'static [u8]) -> Result<(Selection, TfliteModel)> {
        let model = match self.bytes {
            Some(bytes) => TfliteModel::load(bytes)?,
            None => TfliteModel::load(builtin)?,
        };
        Ok((self.selection, model))
    }
}

fn releases() -> &'static str {
    if cfg!(feature = "esp-emulator") {
        "http://192.168.4.1:8765/wake"
    } else {
        RELEASES
    }
}

pub(super) fn catalog() -> Result<Catalog> {
    let body = get(
        &format!("{}/latest/download/metadata.json", releases()),
        CATALOG_LIMIT as usize,
    )?;
    let mut releases = Catalog::parse(&body, RUNTIMES)?;
    releases
        .models
        .retain(|model| model.asset_size() <= MODEL_LIMIT);
    Ok(releases)
}

pub(super) fn download(catalog: &Catalog, id: &str) -> Result<Option<Candidate>> {
    let Some(model) = catalog.models.iter().find(|model| model.selection.id == id) else {
        return Ok(None);
    };
    let selection = model.selection.clone();
    let bytes = if selection.sha256 == crate::model::WAKE_MODEL_SHA {
        None
    } else {
        let url = format!(
            "{}/download/{}/{}",
            releases(),
            catalog.tag,
            model.asset_filename()
        );
        let bytes = get(&url, model.asset_size() as usize)?;
        model.verify_bytes(&bytes)?;
        Some(bytes)
    };
    Ok(Some(Candidate { selection, bytes }))
}

pub(super) fn save_asset(selection: &Selection, interpreter: &TfliteModel) -> Result<()> {
    if selection.sha256 == model::WAKE_MODEL_SHA {
        return Ok(());
    }
    let path = asset_path(selection);
    let directory = path.parent().context("Missing wake model directory")?;
    fs::create_dir_all(directory)?;
    if path.exists() {
        if Candidate::load_saved(selection.clone()).is_ok() {
            return Ok(());
        }
        fs::remove_file(&path)?;
    }
    let temporary = directory.join("incoming.tflite");
    crate::diagnostics::flash_write(|| {
        let mut file = File::create(&temporary)?;
        file.write_all(interpreter.bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, path)
    })?;
    Ok(())
}

pub(super) fn remove_unused(active: &Selection) {
    let directory = Path::new(MOUNT).join("wake");
    let Ok(files) = fs::read_dir(directory) else {
        return;
    };
    let keep = asset_path(active);
    for item in files.flatten() {
        let path = item.path();
        if path != keep
            && path.extension().is_some_and(|kind| kind == "tflite")
            && let Err(error) = fs::remove_file(path)
        {
            log::warn!("Remove previous wake model: {error}");
        }
    }
}

fn asset_path(selection: &Selection) -> PathBuf {
    Path::new(MOUNT)
        .join("wake")
        .join(format!("{}.tflite", selection.sha256))
}

fn get(url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut url = Url::parse(url)?;
    let deadline = Instant::now() + Duration::from_secs(90);
    for _ in 0..6 {
        let fixture = cfg!(feature = "esp-emulator")
            && url.scheme() == "http"
            && url.host_str() == Some("192.168.4.1")
            && url.port() == Some(8765);
        let github = url.scheme() == "https"
            && url.port().is_none()
            && matches!(
                url.host_str(),
                Some(
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            );
        ensure!(
            (fixture || github) && url.username().is_empty() && url.password().is_none(),
            "Invalid wake model download address"
        );
        // ESP-IDF writes the full request line, including GitHub's signed query, at once.
        ensure!(
            url.as_str().len() <= REQUEST_BUFFER - 16,
            "Wake download URL is too long"
        );
        let mut connection = http_client::open(Duration::from_secs(12), Some(REQUEST_BUFFER))?;
        connection
            .initiate_request(Method::Get, url.as_str(), &[("User-Agent", "Goodwin")])
            .with_context(|| {
                format!(
                    "Open wake download at {}",
                    url.host_str().unwrap_or_default()
                )
            })?;
        connection
            .initiate_response()
            .context("Read wake download headers")?;
        let status = connection.status();
        if (300..400).contains(&status) {
            let next = connection
                .header("Location")
                .context("Missing wake model redirect")?;
            url = url.join(next)?;
            ensure!(Instant::now() < deadline, "Wake model download timed out");
            continue;
        }
        ensure!((200..300).contains(&status), "Wake model HTTP {status}");
        let length = connection
            .header("Content-Length")
            .and_then(|value| value.parse::<usize>().ok());
        ensure!(
            length.is_none_or(|length| length <= limit),
            "Wake model download is too large"
        );
        let mut body = Vec::new();
        body.try_reserve_exact(length.unwrap_or(0))?;
        let mut buffer = [0; 2048];
        loop {
            ensure!(Instant::now() < deadline, "Wake model download timed out");
            let count = connection
                .read(&mut buffer)
                .context("Read wake download content")?;
            if count == 0 {
                break;
            }
            ensure!(
                body.len() + count <= limit,
                "Wake model download is too large"
            );
            body.try_reserve(count)?;
            body.extend_from_slice(&buffer[..count]);
        }
        // SAFETY: The connection and its client stay alive throughout the transfer.
        ensure!(
            unsafe { esp_http_client_is_complete_data_received(connection.handle()) },
            "Incomplete wake model download"
        );
        return Ok(body);
    }
    goodwin_error::bail!("Too many wake model redirects")
}

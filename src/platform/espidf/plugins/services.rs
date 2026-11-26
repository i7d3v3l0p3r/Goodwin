use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};

use goodwin_error::{Context, Result, bail, ensure};
use goodwin_plugin::{
    provider::{MAX_CHAT_CHUNK_BYTES, SpeechAudio},
    services::Request,
    settings::SettingsData,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    plugin_wasm::HostServices,
    plugins::{
        ProviderConfig,
        package::{self, Manifest, WasmHttp},
        store::quota::{Quota, file_size},
    },
    voice_core::VoiceRequest,
};

mod http;

const STORAGE_LIMIT: usize = 128 * 1024;

pub(super) struct Host {
    pub(super) data: PathBuf,
    storage: bool,
    quota: Quota,
    http: Option<WasmHttp>,
    has_connections: bool,
    config: Option<ProviderConfig>,
    request: Option<VoiceRequest>,
    deadline: Instant,
    chunks: Option<StreamChunks>,
}

#[derive(Default)]
pub(super) struct Call {
    pub(super) config: Option<ProviderConfig>,
    pub(super) chunks: Option<StreamChunks>,
    pub(super) request: Option<VoiceRequest>,
}

pub(super) enum StreamChunks {
    Chat(SyncSender<String>),
    Speech(SyncSender<SpeechAudio>),
}

impl Host {
    pub(super) fn new(data: PathBuf, manifest: &Manifest) -> Self {
        Self {
            data,
            storage: manifest.wasm_storage,
            quota: Quota::default(),
            http: manifest.wasm_http.clone(),
            has_connections: manifest.provider_connections,
            config: None,
            request: None,
            deadline: Instant::now(),
            chunks: None,
        }
    }

    pub(super) fn begin(&mut self, call: Call, timeout: Duration) -> http::Invocation {
        self.config = call.config;
        self.request = call.request;
        self.deadline = Instant::now() + timeout;
        self.chunks = call.chunks;
        http::Invocation::new()
    }

    pub(super) fn finish(&mut self) {
        self.config = None;
        self.request = None;
        self.chunks = None;
    }

    fn connection(&self) -> Result<&crate::plugins::connections::Connection> {
        match &self.config {
            Some(ProviderConfig::Connection(connection)) => Ok(connection),
            _ => bail!("Select a provider connection"),
        }
    }

    fn settings(&self) -> Result<std::sync::Arc<SettingsData>> {
        match &self.config {
            Some(ProviderConfig::Settings(settings)) => Ok(settings.clone()),
            _ => Ok(SettingsData::read(&self.data)?.into()),
        }
    }

    fn http_call(&self) -> super::super::http_client::HttpCall<'_> {
        super::super::http_client::HttpCall {
            deadline: self.deadline,
            request: self.request.as_ref(),
        }
    }

    fn destination(&self) -> Result<(String, String, bool)> {
        if self.has_connections {
            let connection = self.connection()?;
            return Ok((
                connection.base_url.clone(),
                connection.key.clone(),
                connection.protocol.requires_key(),
            ));
        }
        let http = self
            .http
            .as_ref()
            .context("Plugin does not declare HTTP access")?;
        let data = self.settings()?;
        let base = http
            .base_url
            .clone()
            .or_else(|| {
                data.values
                    .get(http.base_url_setting.as_ref()?)?
                    .as_str()
                    .map(str::to_owned)
            })
            .context("Configure the provider API base URL")?;
        let key = data
            .secrets
            .get(&http.key_setting)
            .cloned()
            .unwrap_or_default();
        Ok((base, key, http.require_key))
    }

    fn path(&self, path: &str) -> Result<PathBuf> {
        ensure!(self.storage, "Plugin does not declare storage access");
        ensure!(
            package::path(path) && path.split('/').count() <= 5,
            "Invalid plugin data path"
        );
        Ok(self.data.join(path))
    }
}

pub(super) fn valid_provider_url(base: &str, key: &str) -> bool {
    crate::plugins::settings::valid_base_url(base)
        && (base.starts_with("https://")
            || cfg!(feature = "esp-emulator")
                && base == "http://192.168.4.1:8765/v1"
                && key.is_empty())
}

impl HostServices for Host {
    fn execute(&mut self, request: Request, input: &[u8]) -> Result<Vec<u8>> {
        self.http_call().check()?;
        match request {
            Request::Settings if self.has_connections => {
                let connection = self.connection()?;
                Ok(goodwin_json::to_vec(
                    &serde_json::json!({"protocol": connection.protocol}),
                )?)
            }
            Request::Settings => {
                let settings = self.settings()?;
                Ok(goodwin_json::to_vec(&settings.values)?)
            }
            Request::HttpStart { request } => {
                let (base, key, require_key) = self.destination()?;
                http::start(request, input, &base, &key, require_key, self.http_call())
            }
            Request::HttpRead { length } => http::read(length, self.http_call()),
            Request::HttpClose => {
                http::close();
                Ok(Vec::new())
            }
            Request::Chat => {
                let Some(StreamChunks::Chat(sender)) = &self.chunks else {
                    bail!("Unexpected chat chunk");
                };
                ensure!(
                    !input.is_empty() && input.len() <= MAX_CHAT_CHUNK_BYTES,
                    "Invalid chat chunk"
                );
                sender
                    .send(std::str::from_utf8(input)?.to_owned())
                    .context("Chat stream closed")?;
                Ok(Vec::new())
            }
            Request::Speech => {
                let Some(StreamChunks::Speech(sender)) = &self.chunks else {
                    bail!("Unexpected speech chunk");
                };
                ensure!(
                    input.len() <= crate::plugin_wasm::META_LIMIT,
                    "Invalid speech chunk"
                );
                sender
                    .send(goodwin_json::from_slice::<SpeechAudio>(input)?)
                    .context("Speech stream closed")?;
                Ok(Vec::new())
            }
            Request::List { path } => {
                let directory = match fs::read_dir(self.path(&path)?) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(b"[]".to_vec());
                    }
                    Err(error) => return Err(error.into()),
                };
                let mut names = Vec::new();
                for entry in directory {
                    if let Ok(name) = entry?.file_name().into_string()
                        && !name.contains('/')
                        && package::path(&name)
                    {
                        names.push(name);
                        ensure!(names.len() <= 2048, "Plugin data directory is too large");
                    }
                }
                let listing = goodwin_json::to_vec(&names)?;
                ensure!(
                    listing.len() <= STORAGE_LIMIT,
                    "Plugin listing is too large"
                );
                Ok(listing)
            }
            Request::Read {
                path,
                offset,
                length,
                optional,
            } => {
                ensure!(length <= STORAGE_LIMIT, "Plugin file read is too large");
                let file = match File::open(self.path(&path)?) {
                    Ok(file) => file,
                    Err(error) if optional && error.kind() == io::ErrorKind::NotFound => {
                        return Ok(Vec::new());
                    }
                    Err(error) => return Err(error.into()),
                };
                let mut data = Vec::new();
                let mut file = file;
                ensure!(
                    file.metadata()?.is_file(),
                    "Plugin data must be a regular file"
                );
                file.seek(SeekFrom::Start(offset))?;
                file.take(length as u64).read_to_end(&mut data)?;
                Ok(data)
            }
            Request::Write { path, replace } => {
                ensure!(
                    input.len() <= STORAGE_LIMIT,
                    "Plugin file write is too large"
                );
                let path = self.path(&path)?;
                ensure!(replace || !path.try_exists()?, "Plugin file already exists");
                let next =
                    self.quota
                        .projected(&self.data, file_size(&path)?, input.len() as u64)?;
                fs::create_dir_all(path.parent().context("Missing plugin directory")?)?;
                if let Err(error) = replace_file(&path, input) {
                    self.quota.invalidate();
                    return Err(error);
                }
                self.quota.saved(next);
                Ok(Vec::new())
            }
            Request::Rename { from, to } => {
                let (from, to) = (self.path(&from)?, self.path(&to)?);
                ensure!(from.is_file(), "Plugin data must be a regular file");
                fs::create_dir_all(to.parent().context("Missing plugin directory")?)?;
                fs::rename(from, to)?;
                self.quota.invalidate();
                Ok(Vec::new())
            }
            Request::Remove { path } => {
                let path = self.path(&path)?;
                let size = file_size(&path)?;
                remove_file(&path)?;
                self.quota.removed(size);
                Ok(Vec::new())
            }
        }
    }
}

pub(crate) fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    read_json_with(path, |file| Ok(serde_json::from_reader(file)?))
}

pub(crate) fn read_json_with<T: Default>(
    path: &Path,
    decode: impl FnOnce(File) -> Result<T>,
) -> Result<T> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let backup = sidecar(path, "bak")?;
            match File::open(backup) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= STORAGE_LIMIT as u64,
        "Plugin JSON file is too large"
    );
    decode(file)
}

pub(crate) fn save_json(path: &Path, value: &impl Serialize, limit: usize) -> Result<()> {
    save_json_bytes(path, &goodwin_json::to_vec(value)?, limit)
}

pub(crate) fn save_json_bytes(path: &Path, bytes: &[u8], limit: usize) -> Result<()> {
    ensure!(bytes.len() <= limit, "Plugin JSON file is too large");
    replace_file(path, bytes)
}

fn sidecar(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Missing data filename")?;
    Ok(path.with_file_name(format!(".{name}.{suffix}")))
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::diagnostics::flash_write(|| write_replacement(path, bytes))
}

fn write_replacement(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = sidecar(path, "new")?;
    let backup = sidecar(path, "bak")?;
    if backup.try_exists()? {
        if path.try_exists()? {
            remove_file(&backup)?;
        } else {
            fs::rename(&backup, path)?;
        }
    }
    let mut pending = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    pending.write_all(bytes)?;
    pending.sync_all()?;
    drop(pending);
    let previous = path.try_exists()?;
    if previous {
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temp, path) {
        if previous {
            fs::rename(&backup, path)?;
        }
        return Err(error.into());
    }
    if previous && let Err(error) = remove_file(&backup) {
        log::warn!("Remove previous plugin data file: {error}");
    }
    Ok(())
}

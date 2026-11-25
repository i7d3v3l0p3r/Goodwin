use std::{
    fs::File,
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, OwnedFd},
    path::{Path, PathBuf},
    sync::LazyLock,
    thread,
    time::Instant,
};

use axum::body::Bytes;
use goodwin_plugin::{
    services::{HttpBody, HttpRequest, HttpResponse, Request},
    settings::SettingsData,
};
use reqwest::{Client, Method, Response, Url, header, multipart, redirect::Policy};
use rustix::fs::{self, AtFlags, Dir, Mode, OFlags};
use tokio::sync::mpsc::error::TrySendError;

use super::*;
use crate::{
    plugin_wasm::{HostServices, META_LIMIT},
    plugins::{connections, package::WasmHttp},
};

const STORAGE_LIMIT: usize = 128 * 1024;
const SPEECH_ERROR_LIMIT: usize = 8 * 1024;
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) struct Host {
    data: PathBuf,
    http: Option<WasmHttp>,
    has_connections: bool,
    connection: Option<connections::Connection>,
    storage: Option<OwnedFd>,
    response: Option<HttpResponseBody>,
    runtime: tokio::runtime::Handle,
    pub(super) chunks: Option<StreamChunks>,
    pub(super) deadline: Instant,
    pub(super) shutdown: Option<watch::Receiver<bool>>,
}

impl Host {
    pub(super) fn new(data: PathBuf, manifest: &Manifest) -> Result<Self> {
        let storage = manifest
            .wasm_storage
            .then(|| fs::open(&data, DIRECTORY_FLAGS, Mode::empty()))
            .transpose()?;
        Ok(Self {
            data,
            http: manifest.wasm_http.clone(),
            has_connections: manifest.provider_connections,
            connection: None,
            storage,
            response: None,
            runtime: tokio::runtime::Handle::current(),
            chunks: None,
            deadline: Instant::now(),
            shutdown: None,
        })
    }

    pub(super) fn finish(&mut self) {
        self.connection = None;
        self.response = None;
        self.chunks = None;
        self.shutdown = None;
    }

    pub(super) fn select_connection(&mut self, id: Option<&str>) -> Result<()> {
        self.connection = if let Some(id) = id {
            ensure!(
                self.has_connections,
                "Plugin does not register provider connections"
            );
            Some(
                connections::Store::read(&self.data)?
                    .connection(id)
                    .context("Provider connection is unavailable")?
                    .clone(),
            )
        } else {
            None
        };
        Ok(())
    }

    fn check_active(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "WASM host operation timed out"
        );
        ensure!(
            self.shutdown
                .as_ref()
                .is_none_or(|shutdown| !*shutdown.borrow()),
            "WASM plugin is stopping"
        );
        Ok(())
    }
}

impl HostServices for Host {
    fn execute(&mut self, request: Request, input: &[u8]) -> Result<Vec<u8>> {
        self.check_active()?;
        match request {
            Request::Settings if self.has_connections => {
                let connection = self
                    .connection
                    .as_ref()
                    .context("Select a provider connection")?;
                Ok(serde_json::to_vec(
                    &json!({"protocol": connection.protocol}),
                )?)
            }
            Request::Settings => Ok(serde_json::to_vec(&SettingsData::read(&self.data)?.values)?),
            Request::HttpStart { request } => self.http_start(request, input),
            Request::HttpRead { length } => {
                ensure!(length <= 64 * 1024, "HTTP chunk is too large");
                let response = self.response.as_mut().context("No open HTTP response")?;
                self.runtime.block_on(wait_for_http(
                    response.read(length),
                    self.deadline,
                    self.shutdown.clone(),
                ))?
            }
            Request::HttpClose => {
                self.response = None;
                Ok(Vec::new())
            }
            Request::Chat => {
                let Some(StreamChunks::Chat(chunks)) = &self.chunks else {
                    bail!("Unexpected chat stream chunk")
                };
                ensure!(
                    !input.is_empty() && input.len() <= MAX_CHAT_CHUNK_BYTES,
                    "Invalid chat stream chunk"
                );
                send_chunk(chunks, std::str::from_utf8(input)?.to_owned(), self)?;
                Ok(Vec::new())
            }
            Request::Speech => {
                let Some(StreamChunks::Speech(chunks)) = &self.chunks else {
                    bail!("Unexpected speech stream chunk")
                };
                ensure!(input.len() <= META_LIMIT, "Invalid speech stream chunk");
                send_chunk(chunks, serde_json::from_slice::<SpeechAudio>(input)?, self)?;
                Ok(Vec::new())
            }
            Request::List { path } => self.list(&path),
            Request::Read {
                path,
                offset,
                length,
                optional,
            } => match self.read(&path, offset, length) {
                Err(error)
                    if optional
                        && error.downcast_ref::<rustix::io::Errno>()
                            == Some(&rustix::io::Errno::NOENT) =>
                {
                    Ok(Vec::new())
                }
                result => result,
            },
            Request::Write { path, replace } => {
                self.write(&path, input, replace)?;
                Ok(Vec::new())
            }
            Request::Rename { from, to } => {
                let (from_dir, from_name) = self.parent(&from, false)?;
                let (to_dir, to_name) = self.parent(&to, true)?;
                fs::renameat(&from_dir, from_name, &to_dir, to_name)?;
                Ok(Vec::new())
            }
            Request::Remove { path } => {
                let (dir, name) = self.parent(&path, false)?;
                match fs::unlinkat(&dir, name, AtFlags::empty()) {
                    Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(Vec::new()),
                    Err(error) => Err(error.into()),
                }
            }
        }
    }
}

impl Host {
    fn http_start(&mut self, request: HttpRequest, input: &[u8]) -> Result<Vec<u8>> {
        static CLIENT: LazyLock<Result<Client, reqwest::Error>> = LazyLock::new(|| {
            Client::builder()
                .tls_backend_preconfigured(crate::http_tls::client_config())
                .connect_timeout(Duration::from_secs(8))
                .redirect(Policy::none())
                .build()
        });
        self.response = None;
        let speech = request.path == ["audio", "speech"] && matches!(request.body, HttpBody::Json);
        let (base, key, require_key) = self.http_destination()?;
        ensure!(
            crate::plugins::settings::valid_base_url(&base),
            "Invalid provider API base URL"
        );
        let mut url = Url::parse(&base)?;
        ensure!(
            (1..=8).contains(&request.path.len())
                && request.path.iter().all(|part| {
                    !part.is_empty()
                        && part.len() <= 64
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
                }),
            "Invalid provider endpoint"
        );
        url.path_segments_mut()
            .map_err(|_| anyhow!("Invalid provider API base URL"))?
            .pop_if_empty()
            .extend(&request.path);
        let client = CLIENT
            .as_ref()
            .map_err(|_| anyhow!("Initialize Goodwin HTTP client"))?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let mut outbound = client
            .request(
                if request.get {
                    Method::GET
                } else {
                    Method::POST
                },
                url,
            )
            .timeout(remaining.min(Duration::from_millis(request.timeout_ms.clamp(1, 180_000))))
            .query(&request.query);
        let key = key.trim();
        ensure!(
            request.get || !require_key || !key.is_empty(),
            "Configure the provider API key"
        );
        if !key.is_empty() {
            ensure!(
                key.bytes().all(|byte| byte.is_ascii_graphic()),
                "Invalid provider key"
            );
            outbound = outbound.bearer_auth(key);
        }
        if let Some(accept) = request.accept {
            ensure!(
                matches!(accept.as_str(), "audio/pcm" | "text/event-stream"),
                "Unsupported provider response type"
            );
            outbound = outbound.header(header::ACCEPT, accept);
        }
        outbound = match request.body {
            HttpBody::Empty => {
                ensure!(input.is_empty(), "Unexpected HTTP request body");
                outbound
            }
            HttpBody::Json => outbound
                .header(header::CONTENT_TYPE, "application/json")
                .body(input.to_vec()),
            HttpBody::Wave { fields } => {
                ensure!(input.len() <= MAX_RECORDING_BYTES, "Recording is too large");
                let mut form = multipart::Form::new().part(
                    "file",
                    multipart::Part::bytes(input.to_vec())
                        .file_name("recording.wav")
                        .mime_str("audio/wav")?,
                );
                for (name, value) in fields {
                    form = form.text(name, value);
                }
                outbound.multipart(form)
            }
        };
        let response = self
            .runtime
            .block_on(wait_for_http(
                outbound.send(),
                self.deadline,
                self.shutdown.clone(),
            ))?
            .map_err(reqwest::Error::without_url)?;
        let head = HttpResponse {
            status: response.status().as_u16(),
            content_type: response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.len() <= 1024)
                .unwrap_or_default()
                .to_owned(),
        };
        if response.status().is_success() {
            self.response = Some(HttpResponseBody {
                response,
                unread: Bytes::new(),
            });
        } else if speech {
            let _ = self.runtime.block_on(wait_for_http(
                log_speech_rejection(response, input),
                self.deadline,
                self.shutdown.clone(),
            ));
        }
        Ok(serde_json::to_vec(&head)?)
    }

    fn http_destination(&self) -> Result<(String, String, bool)> {
        if self.has_connections {
            let connection = self
                .connection
                .as_ref()
                .context("Select a provider connection")?;
            return Ok((
                connection.base_url.clone(),
                connection.key.clone(),
                connection.protocol.requires_key(),
            ));
        }
        let config = self
            .http
            .as_ref()
            .context("Plugin does not declare WASM HTTP access")?;
        let settings = SettingsData::read(&self.data)?;
        let base = config
            .base_url
            .clone()
            .or_else(|| {
                settings
                    .values
                    .get(config.base_url_setting.as_ref()?)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .context("Configure the provider API base URL")?;
        let key = settings
            .secrets
            .get(&config.key_setting)
            .cloned()
            .unwrap_or_default();
        Ok((base, key, config.require_key))
    }

    fn parts<'a>(&self, path: &'a str) -> Result<impl DoubleEndedIterator<Item = &'a str>> {
        ensure!(
            crate::plugins::package::path(path) && path.split('/').count() <= 5,
            "Invalid plugin data path"
        );
        Ok(path.split('/'))
    }

    fn parent<'a>(&self, path: &'a str, create: bool) -> Result<(OwnedFd, &'a str)> {
        let root = self
            .storage
            .as_ref()
            .context("Plugin does not declare WASM storage access")?;
        let mut parts = self.parts(path)?;
        let name = parts.next_back().context("Missing plugin data filename")?;
        let mut dir = fs::openat(root, ".", DIRECTORY_FLAGS, Mode::empty())?;
        for part in parts {
            if create {
                match fs::mkdirat(&dir, part, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            dir = fs::openat(&dir, part, DIRECTORY_FLAGS, Mode::empty())?;
        }
        Ok((dir, name))
    }

    fn list(&self, path: &str) -> Result<Vec<u8>> {
        let directory = (|| -> Result<_> {
            let (parent, name) = self.parent(path, false)?;
            Ok(fs::openat(&parent, name, DIRECTORY_FLAGS, Mode::empty())?)
        })();
        let dir = match directory {
            Ok(dir) => dir,
            Err(error)
                if error.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::NOENT) =>
            {
                return Ok(b"[]".to_vec());
            }
            Err(error) => return Err(error),
        };
        let mut names = Vec::new();
        for entry in Dir::read_from(&dir)? {
            let entry = entry?;
            if let Ok(name) = entry.file_name().to_str()
                && !name.contains('/')
                && crate::plugins::package::path(name)
            {
                names.push(name.to_owned());
                ensure!(names.len() <= 2048, "Plugin data directory is too large");
            }
        }
        let names = serde_json::to_vec(&names)?;
        ensure!(
            names.len() <= STORAGE_LIMIT,
            "Plugin data listing is too large"
        );
        Ok(names)
    }

    fn read(&self, path: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        ensure!(length <= STORAGE_LIMIT, "Plugin file read is too large");
        let (dir, name) = self.parent(path, false)?;
        let fd = fs::openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut file = File::from(fd);
        ensure!(
            file.metadata()?.is_file(),
            "Plugin data must be a regular file"
        );
        file.seek(SeekFrom::Start(offset))?;
        let mut data = Vec::new();
        file.take(length as u64).read_to_end(&mut data)?;
        Ok(data)
    }

    fn write(&self, path: &str, input: &[u8], replace: bool) -> Result<()> {
        ensure!(
            input.len() <= STORAGE_LIMIT,
            "Plugin file write is too large"
        );
        let (dir, name) = self.parent(path, true)?;
        let parent = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
        let mut file = tempfile::NamedTempFile::new_in(&parent)?;
        file.write_all(input)?;
        file.as_file().sync_data()?;
        let path = parent.join(Path::new(name));
        if replace {
            file.persist(path)?;
        } else {
            file.persist_noclobber(path)?;
        }
        fs::fsync(&dir)?;
        Ok(())
    }
}

struct HttpResponseBody {
    response: Response,
    unread: Bytes,
}

impl HttpResponseBody {
    async fn read(&mut self, length: usize) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        while self.unread.is_empty() {
            let Some(chunk) = self
                .response
                .chunk()
                .await
                .map_err(reqwest::Error::without_url)?
            else {
                return Ok(Vec::new());
            };
            self.unread = chunk;
        }
        Ok(self.unread.split_to(length.min(self.unread.len())).to_vec())
    }
}

async fn wait_for_http<T>(
    operation: impl Future<Output = T>,
    deadline: Instant,
    shutdown: Option<watch::Receiver<bool>>,
) -> Result<T> {
    tokio::select! {
        biased;
        () = async {
            if let Some(mut shutdown) = shutdown {
                let _ = shutdown.wait_for(|stopping| *stopping).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => bail!("WASM plugin is stopping"),
        result = tokio::time::timeout_at(deadline.into(), operation) => {
            result.context("WASM host operation timed out")
        }
    }
}

async fn log_speech_rejection(response: Response, input: &[u8]) {
    let status = response.status().as_u16();
    let trace = ["x-generation-id", "x-request-id", "cf-ray"]
        .into_iter()
        .filter_map(|name| response.headers().get(name)?.to_str().ok())
        .find(|value| safe_log_id(value))
        .unwrap_or("unavailable")
        .to_owned();
    let request: Value = serde_json::from_slice(input).unwrap_or_default();
    let model = request["model"]
        .as_str()
        .filter(|model| safe_log_id(model))
        .unwrap_or("unknown");
    let input_chars = request["input"]
        .as_str()
        .map_or(0, |text| text.chars().count());
    let mut body = HttpResponseBody {
        response,
        unread: Bytes::new(),
    };
    let reason = match read_speech_rejection(&mut body).await {
        Ok(error) => speech_rejection_reason(&error),
        Err(_) => "unavailable",
    };
    log::warn!(
        "Speech upstream rejected request status={status} model={model} input_chars={input_chars} reason={reason} trace={trace}"
    );
}

async fn read_speech_rejection(body: &mut HttpResponseBody) -> Result<Value> {
    let mut bytes = Vec::new();
    while bytes.len() < SPEECH_ERROR_LIMIT {
        let chunk = body.read(SPEECH_ERROR_LIMIT - bytes.len()).await?;
        if chunk.is_empty() {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

fn safe_log_id(value: &str) -> bool {
    (1..=160).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

fn speech_rejection_reason(body: &Value) -> &'static str {
    let error = &body["error"];
    let message = error["message"].as_str().unwrap_or_default();
    let details = format!("{} {}", message, error["metadata"]["raw"]).to_ascii_lowercase();
    for (reason, patterns) in [
        (
            "input_too_long",
            ["input too long", "input is too long", "maximum length"],
        ),
        (
            "invalid_voice",
            ["invalid voice", "unsupported voice", "voice not found"],
        ),
        (
            "invalid_markup",
            ["invalid ssml", "invalid xml", "ssml parsing"],
        ),
        (
            "content_filter",
            ["content_filter", "content filter", "content policy"],
        ),
        (
            "unsupported_format",
            [
                "unsupported response format",
                "unsupported audio format",
                "invalid response_format",
            ],
        ),
    ] {
        if patterns
            .into_iter()
            .any(|pattern| details.contains(pattern))
        {
            return reason;
        }
    }
    if message.is_empty() || message == "HTTP 400" {
        "unspecified"
    } else {
        "provider_error"
    }
}

fn send_chunk<T>(sender: &mpsc::Sender<T>, mut chunk: T, host: &Host) -> Result<()> {
    loop {
        match sender.try_send(chunk) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Closed(_)) => bail!("Plugin stream was closed"),
            Err(TrySendError::Full(value)) => chunk = value,
        }
        host.check_active()?;
        thread::sleep(Duration::from_millis(5));
    }
}

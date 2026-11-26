use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use goodwin_error::{Context, Result, ensure};
use goodwin_plugin::{
    provider::MAX_RECORDING_BYTES,
    services::{HttpBody, HttpRequest, HttpResponse},
};
use url::Url;

use super::super::super::http_client::{HttpCall, ProviderHttp};

thread_local! {
    // ESP-IDF connections stay on the calling task for the entire guest invocation.
    static RESPONSE: RefCell<Option<ProviderHttp>> = const { RefCell::new(None) };
}

pub(crate) struct Invocation;

impl Invocation {
    pub(super) fn new() -> Self {
        close();
        Self
    }
}

impl Drop for Invocation {
    fn drop(&mut self) {
        close();
    }
}

pub(super) fn close() {
    RESPONSE.with(|response| response.borrow_mut().take());
}

pub(super) fn read(length: usize, call: HttpCall<'_>) -> Result<Vec<u8>> {
    ensure!(length <= 64 * 1024, "HTTP chunk is too large");
    let length = length.min(4096);
    RESPONSE.with(|response| {
        let mut response = response.borrow_mut();
        let connection = response.as_mut().context("No open HTTP response")?;
        let mut bytes = vec![0; length];
        let count = connection.read(&mut bytes, call)?;
        bytes.truncate(count);
        Ok(bytes)
    })
}

pub(super) fn start(
    request: HttpRequest,
    input: &[u8],
    base: &str,
    key: &str,
    require_key: bool,
    call: HttpCall<'_>,
) -> Result<Vec<u8>> {
    close();
    ensure!(
        super::valid_provider_url(base, key),
        "The provider base URL must be HTTPS"
    );
    let mut url = Url::parse(base)?;
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
        .map_err(|_| goodwin_error::report!("Invalid provider API base URL"))?
        .pop_if_empty()
        .extend(&request.path);
    if !request.query.is_empty() {
        url.query_pairs_mut().extend_pairs(&request.query);
    }
    let key = key.trim();
    ensure!(
        request.get || !require_key || !key.is_empty(),
        "Configure the provider API key"
    );
    ensure!(
        key.bytes().all(|byte| byte.is_ascii_graphic()),
        "Invalid provider key"
    );
    let authorization = format!("Bearer {key}");
    let mut headers = Vec::new();
    if !key.is_empty() {
        headers.push(("Authorization", authorization.as_str()));
    }
    if let Some(accept) = &request.accept {
        ensure!(
            matches!(accept.as_str(), "audio/pcm" | "text/event-stream"),
            "Unsupported provider response type"
        );
        headers.push(("Accept", accept.as_str()));
    }
    let mut prefix = String::new();
    let mut suffix = String::new();
    let content_type;
    match request.body {
        HttpBody::Empty => ensure!(input.is_empty(), "Unexpected HTTP request body"),
        HttpBody::Json => headers.push(("Content-Type", "application/json")),
        HttpBody::Wave { fields } => {
            ensure!(input.len() <= MAX_RECORDING_BYTES, "Recording is too large");
            let mut salt = [0; 8];
            getrandom::fill(&mut salt)
                .map_err(|_| goodwin_error::report!("Generate multipart boundary"))?;
            let boundary = format!("goodwin-{:016x}", u64::from_le_bytes(salt));
            for (name, value) in fields {
                ensure!(
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                        && !value.contains(['\r', '\n']),
                    "Invalid multipart field"
                );
                use std::fmt::Write;
                write!(
                    prefix,
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )?;
            }
            use std::fmt::Write;
            write!(
                prefix,
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"recording.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
            )?;
            suffix = format!("\r\n--{boundary}--\r\n");
            content_type = format!("multipart/form-data; boundary={boundary}");
            headers.push(("Content-Type", content_type.as_str()));
        }
    }
    let length = (prefix.len() + input.len() + suffix.len()).to_string();
    if !request.get {
        headers.push(("Content-Length", length.as_str()));
    }
    call.check()?;
    let remaining = call.deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "WASM provider call timed out");
    let timeout = remaining.min(Duration::from_millis(request.timeout_ms.clamp(1, 15_000)));
    let (connection, status) = ProviderHttp::start(
        url.as_str(),
        request.get,
        &headers,
        &[prefix.as_bytes(), input, suffix.as_bytes()],
        timeout,
        call,
    )?;
    let content_type = connection.content_type().unwrap_or_default().to_owned();
    if (200..300).contains(&status) {
        if content_type
            .split(';')
            .next()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("text/event-stream"))
        {
            connection.set_timeout(200)?;
        }
        RESPONSE.with(|response| response.replace(Some(connection)));
    }
    Ok(goodwin_json::to_vec(&HttpResponse {
        status,
        content_type,
    })?)
}

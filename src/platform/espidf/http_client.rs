#[cfg(feature = "wake-tflite")]
use esp_idf_svc::http::client::{Configuration, EspHttpConnection, FollowRedirectsPolicy};
use esp_idf_svc::sys::{self, esp, esp_crt_bundle_attach};
use goodwin_error::{Context, Result, ensure};
use std::{
    ffi::{CStr, CString},
    time::{Duration, Instant},
};

use crate::{builtin_assistant::HttpTransport, voice_core::VoiceRequest};

pub(super) struct HttpClient<'a>(pub(super) Option<&'a VoiceRequest>);

#[derive(Clone, Copy)]
pub(super) struct HttpCall<'a> {
    pub(super) deadline: Instant,
    pub(super) request: Option<&'a VoiceRequest>,
}

impl HttpCall<'_> {
    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            self.request
                .is_none_or(|request| request.failure().is_none()),
            "Voice request cancelled"
        );
        ensure!(
            Instant::now() < self.deadline,
            "Provider HTTP request timed out"
        );
        Ok(())
    }
}

// An owned client keeps response metadata across ESP-IDF's EAGAIN returns.
// All operations and callbacks execute on the same task as the request.
pub(super) struct ProviderHttp {
    raw: sys::esp_http_client_handle_t,
    headers: Box<ResponseHeaders>,
}

#[derive(Default)]
struct ResponseHeaders {
    content_type: String,
}

impl ProviderHttp {
    pub(super) fn start(
        url: &str,
        get: bool,
        headers: &[(&str, &str)],
        body: &[&[u8]],
        timeout: Duration,
        call: HttpCall<'_>,
    ) -> Result<(Self, u16)> {
        call.check()?;
        let url = CString::new(url)?;
        let mut response = Box::<ResponseHeaders>::default();
        // SAFETY: init copies the URL. The callback data has a stable address until cleanup.
        let raw = unsafe {
            sys::esp_http_client_init(&sys::esp_http_client_config_t {
                url: url.as_ptr(),
                method: if get {
                    sys::esp_http_client_method_t_HTTP_METHOD_GET
                } else {
                    sys::esp_http_client_method_t_HTTP_METHOD_POST
                },
                timeout_ms: timeout.as_millis().max(1) as i32,
                buffer_size: 4096,
                disable_auto_redirect: true,
                crt_bundle_attach: Some(esp_crt_bundle_attach),
                event_handler: Some(response_header),
                user_data: std::ptr::from_mut(&mut *response).cast(),
                ..Default::default()
            })
        };
        ensure!(!raw.is_null(), "Initialize provider HTTP client");
        let client = Self {
            raw,
            headers: response,
        };
        let mut length = 0;
        for &(name, value) in headers {
            if name.eq_ignore_ascii_case("Content-Length") {
                length = value
                    .parse::<i32>()
                    .context("Invalid provider request length")?;
                continue;
            }
            let name = CString::new(name)?;
            let value = CString::new(value)?;
            // SAFETY: the live client copies both NUL-terminated header strings.
            esp!(unsafe { sys::esp_http_client_set_header(raw, name.as_ptr(), value.as_ptr()) })?;
        }
        ensure!(
            length >= 0 && length as usize == body.iter().map(|part| part.len()).sum::<usize>(),
            "Invalid provider request length"
        );
        // SAFETY: this task owns the live client and writes at most the declared length.
        esp!(unsafe { sys::esp_http_client_open(raw, length) })?;
        for mut part in body.iter().copied() {
            while !part.is_empty() {
                call.check()?;
                let block = &part[..part.len().min(4096)];
                let sent = unsafe {
                    sys::esp_http_client_write(raw, block.as_ptr().cast(), block.len() as i32)
                };
                ensure!(
                    sent > 0 && sent as usize <= block.len(),
                    "Send provider HTTP request"
                );
                part = &part[sent as usize..];
            }
        }
        if call.request.is_some() {
            client.set_timeout(250)?;
        }
        let mut status = None;
        loop {
            call.check()?;
            // SAFETY: the live client has sent its request. Each retry resumes the parser.
            let result = unsafe { sys::esp_http_client_fetch_headers(raw) };
            let code = unsafe { sys::esp_http_client_get_status_code(raw) };
            if code > 0 {
                // ESP-IDF clears the status field before retrying a partial header read.
                status = Some(code as u16);
            }
            if result >= 0 {
                return Ok((client, status.context("Missing provider HTTP status")?));
            }
            ensure!(
                call.request.is_some() && result == -i64::from(sys::ESP_ERR_HTTP_EAGAIN),
                "Read provider HTTP headers"
            );
        }
    }

    pub(super) fn content_type(&self) -> Option<&str> {
        (!self.headers.content_type.is_empty()).then_some(self.headers.content_type.as_str())
    }

    pub(super) fn content_length(&self) -> Option<usize> {
        // SAFETY: the client is alive and has fetched its headers.
        usize::try_from(unsafe { sys::esp_http_client_get_content_length(self.raw) }).ok()
    }

    pub(super) fn set_timeout(&self, ms: i32) -> Result<()> {
        // SAFETY: this task owns the live client.
        Ok(esp!(unsafe {
            sys::esp_http_client_set_timeout_ms(self.raw, ms)
        })?)
    }

    pub(super) fn read(&mut self, data: &mut [u8], call: HttpCall<'_>) -> Result<usize> {
        loop {
            call.check()?;
            // SAFETY: the connected client writes at most data.len() bytes.
            let count = unsafe {
                sys::esp_http_client_read(self.raw, data.as_mut_ptr().cast(), data.len() as i32)
            };
            if count == -sys::ESP_ERR_HTTP_EAGAIN {
                continue;
            }
            ensure!(count >= 0, "Read provider HTTP body");
            ensure!(
                count != 0
                    || data.is_empty()
                    || unsafe { sys::esp_http_client_is_complete_data_received(self.raw) },
                "Incomplete provider HTTP body"
            );
            return Ok(count as usize);
        }
    }
}

impl Drop for ProviderHttp {
    fn drop(&mut self) {
        // SAFETY: this is the sole owner and callback data is still alive during cleanup.
        unsafe { sys::esp_http_client_cleanup(self.raw) };
    }
}

unsafe extern "C" fn response_header(event: *mut sys::esp_http_client_event_t) -> sys::esp_err_t {
    // SAFETY: ESP-IDF passes an initialized event and NUL-terminated header strings.
    // user_data points to the response headers owned by ProviderHttp.
    unsafe {
        let event = &*event;
        if event.event_id == sys::esp_http_client_event_id_t_HTTP_EVENT_ON_HEADER
            && CStr::from_ptr(event.header_key)
                .to_bytes()
                .eq_ignore_ascii_case(b"content-type")
        {
            let value = CStr::from_ptr(event.header_value).to_bytes();
            let content_type = &mut (*event.user_data.cast::<ResponseHeaders>()).content_type;
            content_type.clear();
            if value.len() <= 1024
                && let Ok(value) = std::str::from_utf8(value)
            {
                content_type.push_str(value);
            }
        }
    }
    sys::ESP_OK
}

#[cfg(feature = "wake-tflite")]
pub(super) fn open(timeout: Duration, tx_buffer_size: Option<usize>) -> Result<EspHttpConnection> {
    Ok(EspHttpConnection::new(&Configuration {
        buffer_size: Some(4096),
        buffer_size_tx: tx_buffer_size,
        timeout: Some(timeout),
        follow_redirects_policy: FollowRedirectsPolicy::FollowNone,
        crt_bundle_attach: Some(esp_crt_bundle_attach),
        ..Default::default()
    })?)
}

impl HttpTransport for HttpClient<'_> {
    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[&[u8]],
        limit: usize,
        mut chunk: impl FnMut(Option<&str>, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let call = HttpCall {
            deadline: Instant::now() + Duration::from_secs(60),
            request: self.0,
        };
        let (mut connection, status) =
            ProviderHttp::start(url, false, headers, body, Duration::from_secs(15), call)?;
        ensure!((200..300).contains(&status), "HTTP {status}");
        let content_type = connection.content_type().map(str::to_owned);
        let length = connection.content_length();
        ensure!(
            length.is_none_or(|length| length <= limit),
            "response is too large"
        );
        chunk(content_type.as_deref(), &[])?;
        let mut received = 0;
        let mut block = [0_u8; 4096];
        loop {
            let count = connection.read(&mut block, call)?;
            if count == 0 {
                break;
            }
            received += count;
            ensure!(received <= limit, "response is too large");
            chunk(content_type.as_deref(), &block[..count])?;
        }
        Ok(())
    }
}

use core::{error::Error, fmt};
use goodwin_error::{Context, Result};
use goodwin_plugin::provider::ProviderHttpError;

use crate::Config;

#[derive(Clone, Copy)]
pub(super) enum Method {
    Get,
    Post,
}

#[derive(Debug)]
pub enum SendError {
    Http(ProviderHttpError),
    Request(goodwin_error::Error),
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(_) => formatter.write_str("Upstream HTTP request failed"),
            Self::Request(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for SendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Request(error) => error.as_error().source(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use std::time::Duration;

    use goodwin_error::ensure;
    use reqwest::{Url, blocking::multipart, header, redirect::Policy};

    use super::*;

    pub use reqwest::blocking::{Client, RequestBuilder, Response};

    pub fn client() -> Result<Client> {
        let mut builder = Client::builder();
        if let Ok(proxy) = std::env::var("GOODWIN_PLUGIN_PROXY") {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        Ok(builder
            .tls_backend_preconfigured(crate::http_tls::client_config())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(50))
            .redirect(Policy::none())
            .build()?)
    }

    pub fn request(
        client: &Client,
        config: &Config,
        method: Method,
        endpoint: &[&str],
    ) -> Result<RequestBuilder> {
        let base = &config.base_url;
        ensure!(base.len() <= 2048, "API base URL is too long");
        let mut url = Url::parse(base).context("Enter an absolute API base URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Enter an HTTP(S) API base URL without credentials, query or fragment"
        );
        url.path_segments_mut()
            .map_err(|_| goodwin_error::report!("Invalid API base URL"))?
            .pop_if_empty()
            .extend(endpoint);
        let request = client.request(
            match method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
            },
            url,
        );
        let key = config.key.trim();
        if key.is_empty() {
            return Ok(request);
        }
        ensure!(
            key.bytes().all(|byte| byte.is_ascii_graphic()),
            "Invalid API key"
        );
        Ok(request.bearer_auth(key))
    }

    pub fn accept(request: RequestBuilder, accept: &'static str) -> RequestBuilder {
        request.header(header::ACCEPT, accept)
    }

    pub fn json(request: RequestBuilder, body: Vec<u8>) -> RequestBuilder {
        request
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
    }

    pub fn wave(
        request: RequestBuilder,
        wav: Vec<u8>,
        fields: Vec<(String, String)>,
    ) -> Result<RequestBuilder> {
        let mut form = multipart::Form::new();
        for (name, value) in fields {
            form = form.text(name, value);
        }
        Ok(request.multipart(
            form.part(
                "file",
                multipart::Part::bytes(wav)
                    .file_name("recording.wav")
                    .mime_str("audio/wav")?,
            ),
        ))
    }

    pub fn send(request: RequestBuilder) -> Result<Response, SendError> {
        let response = request
            .send()
            .map_err(|error| SendError::Request(error.without_url().into()))?;
        if !response.status().is_success() {
            return Err(SendError::Http(ProviderHttpError(
                response.status().as_u16(),
            )));
        }
        Ok(response)
    }

    pub fn content_type(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use std::{io, time::Duration};

    use goodwin_plugin::services::{HttpBody, HttpRequest, HttpResponse, Request, guest};

    use super::*;

    #[derive(Clone)]
    pub struct Client;

    pub struct RequestBuilder {
        request: HttpRequest,
        body: Vec<u8>,
    }

    impl RequestBuilder {
        pub fn timeout(mut self, duration: Duration) -> Self {
            self.request.timeout_ms = duration.as_millis() as u64;
            self
        }

        pub fn query(mut self, query: &[(&str, &str)]) -> Self {
            self.request.query.extend(
                query
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string())),
            );
            self
        }
    }

    pub struct Response(HttpResponse);

    impl io::Read for Response {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            guest::http_read(buffer)
        }
    }

    impl Drop for Response {
        fn drop(&mut self) {
            let _ = guest::call(Request::HttpClose, &[], &mut []);
        }
    }

    pub fn client() -> Result<Client> {
        Ok(Client)
    }

    pub fn request(
        _: &Client,
        _: &Config,
        method: Method,
        endpoint: &[&str],
    ) -> Result<RequestBuilder> {
        Ok(RequestBuilder {
            request: HttpRequest {
                path: endpoint.iter().map(|part| part.to_string()).collect(),
                get: matches!(method, Method::Get),
                query: Vec::new(),
                accept: None,
                timeout_ms: 50_000,
                body: HttpBody::Empty,
            },
            body: Vec::new(),
        })
    }

    pub fn accept(mut request: RequestBuilder, accept: &'static str) -> RequestBuilder {
        request.request.accept = Some(accept.to_owned());
        request
    }

    pub fn json(mut request: RequestBuilder, body: Vec<u8>) -> RequestBuilder {
        request.request.body = HttpBody::Json;
        request.body = body;
        request
    }

    pub fn wave(
        mut request: RequestBuilder,
        wav: Vec<u8>,
        fields: Vec<(String, String)>,
    ) -> Result<RequestBuilder> {
        request.request.body = HttpBody::Wave { fields };
        request.body = wav;
        Ok(request)
    }

    pub fn send(request: RequestBuilder) -> Result<Response, SendError> {
        let mut head = [0; 4096];
        let length = guest::call(
            Request::HttpStart {
                request: request.request,
            },
            &request.body,
            &mut head,
        )
        .context("Goodwin provider HTTP request failed")
        .map_err(SendError::Request)?;
        let head: HttpResponse = goodwin_plugin::json_codec::from_slice(&head[..length])
            .map_err(|error| SendError::Request(error.into()))?;
        if !(200..300).contains(&head.status) {
            return Err(SendError::Http(ProviderHttpError(head.status)));
        }
        Ok(Response(head))
    }

    pub fn content_type(response: &Response) -> Option<&str> {
        Some(&response.0.content_type)
    }
}

pub(super) use platform::*;

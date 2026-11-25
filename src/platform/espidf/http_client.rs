use anyhow::{Result, ensure};
use esp_idf_svc::{
    handle::RawHandle,
    http::{
        Method,
        client::{Configuration, EspHttpConnection, FollowRedirectsPolicy},
    },
    sys::{esp_crt_bundle_attach, esp_http_client_is_complete_data_received},
};
use std::time::{Duration, Instant};

use crate::builtin_assistant::HttpTransport;

pub(super) struct HttpClient;

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

impl HttpTransport for HttpClient {
    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[&[u8]],
        limit: usize,
        mut chunk: impl FnMut(Option<&str>, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let mut connection = open(Duration::from_secs(15), None)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        connection.initiate_request(Method::Post, url, headers)?;
        for part in body {
            for block in part.chunks(4096) {
                ensure!(Instant::now() < deadline, "Timed out sending request");
                connection.write_all(block)?;
            }
        }
        connection.initiate_response()?;
        let status = connection.status();
        ensure!((200..300).contains(&status), "HTTP {status}");
        let content_type = connection.header("Content-Type").map(str::to_owned);
        let length = connection
            .header("Content-Length")
            .and_then(|length| length.parse::<usize>().ok());
        ensure!(
            length.is_none_or(|length| length <= limit),
            "response is too large"
        );
        chunk(content_type.as_deref(), &[])?;
        let mut received = 0;
        let mut block = [0_u8; 4096];
        loop {
            ensure!(Instant::now() < deadline, "Timed out reading response");
            let count = connection.read(&mut block)?;
            if count == 0 {
                break;
            }
            received += count;
            ensure!(received <= limit, "response is too large");
            chunk(content_type.as_deref(), &block[..count])?;
        }
        // SAFETY: The connection owns a valid client handle for the duration of the call.
        ensure!(
            unsafe { esp_http_client_is_complete_data_received(connection.handle()) },
            "Incomplete response"
        );
        Ok(())
    }
}

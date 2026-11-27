use base64::{Engine, engine::general_purpose};
use goodwin_error::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use url::{Host, Position, Url};

const BODY_LIMIT: usize = 128 * 1024;
const TIMEOUT: Duration = Duration::from_secs(12);
pub const DEFAULT_PROBE: &str = "https://www.gstatic.com/generate_204";

#[derive(Serialize, Deserialize)]
pub enum Request {
    Resolve(String),
    Probe {
        socket: PathBuf,
        password: String,
        url: String,
    },
}

pub fn web_url(input: &str) -> Result<Url> {
    ensure!(
        input.len() <= 4096 && !input.chars().any(char::is_control),
        "Invalid URL"
    );
    let url = Url::parse(input)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none(),
        "Expected HTTP(S) URL"
    );
    Ok(url)
}

// This entry point runs only in the disposable helper. Even a slow DNS lookup
// or a peer trickling one byte per read cannot hold the monitor indefinitely.
pub fn serve() -> Result<()> {
    // SAFETY: alarm sets the timeout for this single-purpose process.
    unsafe { libc::alarm(25) };
    let request = serde_json::from_reader(std::io::stdin().lock().take(16384))?;
    let links = match request {
        Request::Resolve(source) => resolve(&source)?,
        Request::Probe {
            socket,
            password,
            url,
        } => {
            probe(&socket, &password, &url)?;
            Vec::new()
        }
    };
    serde_json::to_writer(std::io::stdout().lock(), &links)?;
    Ok(())
}

fn links(body: &str) -> Result<Vec<String>> {
    let body = body.trim_start_matches('\u{feff}').trim();
    let decoded;
    let text = if body.contains("://") {
        body
    } else {
        let encoded: Vec<_> = body.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        decoded = [
            general_purpose::STANDARD,
            general_purpose::STANDARD_NO_PAD,
            general_purpose::URL_SAFE,
            general_purpose::URL_SAFE_NO_PAD,
        ]
        .iter()
        .find_map(|engine| engine.decode(&encoded).ok())
        .context("Invalid subscription")?;
        std::str::from_utf8(&decoded)?
    };
    let mut result = Vec::new();
    for link in text
        .lines()
        .map(str::trim)
        .filter(|s| s.starts_with("vless://") && s.len() <= 4096)
    {
        if !result.iter().any(|s| s == link) {
            result.push(link.to_owned());
        }
        if result.len() == 32 {
            break;
        }
    }
    ensure!(!result.is_empty(), "No VLESS profiles");
    Ok(result)
}

fn resolve(source: &str) -> Result<Vec<String>> {
    let mut url = web_url(source)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    for _ in 0..=5 {
        let mut res = agent.get(url.as_str()).call()?;
        if matches!(res.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = res
                .headers()
                .get("location")
                .context("Missing redirect")?
                .to_str()?;
            if location.starts_with("vless://") {
                return links(location);
            }
            let next = web_url(url.join(location)?.as_str())?;
            ensure!(
                url.scheme() != "https" || next.scheme() == "https",
                "Insecure redirect"
            );
            url = next;
            continue;
        }
        ensure!(res.status().is_success(), "Subscription unavailable");
        let mut text = String::new();
        res.body_mut()
            .as_reader()
            .take(BODY_LIMIT as u64 + 1)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= BODY_LIMIT, "Subscription too large");
        return links(&text);
    }
    bail!("Too many subscription redirects")
}

fn expect(stream: &mut UnixStream, bytes: &[u8; 2]) -> Result<()> {
    let mut reply = [0; 2];
    stream.read_exact(&mut reply)?;
    ensure!(reply == *bytes, "SOCKS negotiation failed");
    Ok(())
}

fn tunnel(socket: &Path, password: &str, url: &Url) -> Result<UnixStream> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(&[5, 1, 2])?;
    expect(&mut stream, &[5, 2])?;
    let mut auth = b"\x01\x07goodwin".to_vec();
    auth.push(password.len().try_into()?);
    auth.extend_from_slice(password.as_bytes());
    stream.write_all(&auth)?;
    expect(&mut stream, &[1, 0])?;
    let mut connect = vec![5, 1, 0];
    match url.host().context("Missing host")? {
        Host::Domain(domain) => {
            connect.extend([3, domain.len().try_into()?]);
            connect.extend(domain.bytes());
        }
        Host::Ipv4(ip) => {
            connect.push(1);
            connect.extend(ip.octets());
        }
        Host::Ipv6(ip) => {
            connect.push(4);
            connect.extend(ip.octets());
        }
    }
    connect.extend(
        url.port_or_known_default()
            .context("Missing port")?
            .to_be_bytes(),
    );
    stream.write_all(&connect)?;
    // All Goodwin workers return the fixed IPv4 BND.ADDR 0.0.0.0:0.
    let mut reply = [0; 10];
    stream.read_exact(&mut reply)?;
    ensure!(reply[..4] == [5, 0, 0, 1], "Tunnel unavailable");
    Ok(stream)
}

fn check_http(mut stream: impl Read + Write, url: &Url) -> Result<()> {
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        &url[Position::BeforePath..Position::AfterQuery],
        &url[Position::BeforeHost..Position::AfterPort]
    )?;
    stream.flush()?;
    let mut status = String::new();
    BufReader::new(stream).take(1024).read_line(&mut status)?;
    let mut parts = status.split_ascii_whitespace();
    ensure!(
        status.ends_with("\r\n")
            && matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1"))
            && parts
                .next()
                .and_then(|s| s.parse::<u16>().ok())
                .is_some_and(|n| (200..300).contains(&n)),
        "Probe failed"
    );
    Ok(())
}

fn probe(socket: &Path, password: &str, target: &str) -> Result<()> {
    let url = web_url(target)?;
    let stream = tunnel(socket, password, &url)?;
    if url.scheme() == "http" {
        return check_http(stream, &url);
    }
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let host = url
        .host_str()
        .context("Missing TLS host")?
        .trim_matches(['[', ']'])
        .to_owned();
    let tls = rustls::ClientConnection::new(Arc::new(config), host.try_into()?)?;
    check_http(rustls::StreamOwned::new(tls, stream), &url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_accepts_plain_and_encoded_lists_in_provider_order() {
        let list = "trojan://other\r\nvless://first\r\nvless://second\nvless://first\n";
        let expected = ["vless://first", "vless://second"];
        assert_eq!(links(list).unwrap(), expected);
        for codec in [general_purpose::STANDARD, general_purpose::URL_SAFE_NO_PAD] {
            assert_eq!(links(&codec.encode(list)).unwrap(), expected);
        }
        assert!(links("<html>sign in</html>").is_err());
        assert!(web_url("file:///tmp/profile").is_err());
    }
}

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use goodwin_error::{Context, Result, bail, ensure, report};
use std::collections::BTreeMap;
use url::Url;

use crate::types::{Profile, Reality, Transport};

pub fn outbound(link: &str) -> Result<Profile> {
    ensure!(link.len() <= 4096, "Profile is too long");
    let url = Url::parse(link).map_err(|_| report!("Invalid share link"))?;
    ensure!(
        url.scheme() == "vless" && url.password().is_none() && matches!(url.path(), "" | "/"),
        "Invalid share link"
    );
    let id = url.username();
    ensure!(
        id.len() == 36
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }),
        "Invalid account"
    );
    let host = url.host().context("Missing server")?.to_string();
    let server = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = url.port().filter(|p| *p != 0).context("Missing port")?;
    let mut fields = BTreeMap::new();
    for (k, v) in url.query_pairs() {
        ensure!(
            fields.insert(k.into_owned(), v.into_owned()).is_none(),
            "Repeated field"
        );
    }
    let mut take = |k: &str, default: &str| fields.remove(k).unwrap_or_else(|| default.into());
    let transport = match take("type", "tcp").as_str() {
        "raw" | "tcp" => Transport::Tcp,
        "ws" => Transport::WebSocket,
        "httpupgrade" => Transport::HttpUpgrade,
        "http" | "h2" => Transport::Http2,
        "grpc" => Transport::Grpc,
        "xhttp" => Transport::Xhttp,
        _ => bail!("Unsupported VLESS transport"),
    };
    let security = take("security", "none");
    ensure!(
        matches!(security.as_str(), "none" | "tls" | "reality"),
        "Unsupported security"
    );
    let mut tls_name = None;
    let mut fingerprint = None;
    let mut alpn = Vec::new();
    if security != "none" {
        let sni = take("sni", &server);
        tls_name = Some(if sni.is_empty() { server.clone() } else { sni });
        let fp = take("fp", if security == "reality" { "chrome" } else { "" });
        fingerprint = (!fp.is_empty()).then_some(fp);
        let list = take("alpn", "");
        if !list.is_empty() {
            for protocol in list.split(',') {
                ensure!(
                    !protocol.is_empty()
                        && protocol.len() <= 255
                        && protocol.bytes().all(|b| b.is_ascii_graphic()),
                    "Invalid ALPN"
                );
                alpn.push(protocol.to_owned());
            }
        }
    }
    let reality = if security == "reality" {
        let key = URL_SAFE_NO_PAD
            .decode(take("pbk", ""))?
            .try_into()
            .map_err(|_| report!("Invalid REALITY key"))?;
        let sid = take("sid", "");
        ensure!(
            sid.len() <= 16
                && sid.len().is_multiple_of(2)
                && sid.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid short ID"
        );
        let short_id = (0..sid.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&sid[i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()?;
        ensure!(
            matches!(take("spx", "/").as_str(), "" | "/"),
            "Unsupported REALITY path"
        );
        Some(Reality { key, short_id })
    } else {
        None
    };
    let flow = take("flow", "");
    let vision = !flow.is_empty();
    ensure!(
        !vision
            || (flow == "xtls-rprx-vision" && transport == Transport::Tcp && tls_name.is_some()),
        "Unsupported flow"
    );
    let encryption = take("encryption", "none");
    ensure!(
        encryption == "none" || encryption.starts_with("mlkem768x25519plus."),
        "Unsupported encryption"
    );
    for flag in ["allowInsecure", "insecure"] {
        ensure!(
            matches!(take(flag, "0").as_str(), "0" | "false"),
            "Unsupported TLS verification setting"
        );
    }
    ensure!(
        take("headerType", "none") == "none",
        "Unsupported TCP header"
    );
    let default_authority = tls_name.as_deref().unwrap_or(&host);
    let mut authority = if default_authority.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{default_authority}]")
    } else {
        default_authority.to_owned()
    };
    let fallback_authority = authority.clone();
    let mut path = String::from("/");
    match transport {
        Transport::Tcp => {}
        Transport::Grpc => {
            ensure!(take("mode", "gun") == "gun", "Unsupported gRPC mode");
            authority = take("authority", &fallback_authority);
            let service = take("serviceName", "");
            path = if service.starts_with('/') {
                service.split('|').next().unwrap_or_default().to_owned()
            } else {
                format!("/{service}/Tun")
            };
        }
        _ => {
            authority = take("host", &fallback_authority);
            if transport == Transport::Http2 {
                for entry in authority.split(',') {
                    entry.parse::<http::uri::Authority>()?;
                }
                authority.truncate(authority.find(',').unwrap_or(authority.len()));
            }
            path = take("path", "/");
            if transport == Transport::Xhttp {
                let mode = take("mode", "auto");
                ensure!(
                    mode == "stream-one"
                        || reality.is_some() && matches!(mode.as_str(), "" | "auto"),
                    "Unsupported XHTTP mode"
                );
            }
        }
    }
    if authority.is_empty() {
        authority = fallback_authority;
    }
    if transport != Transport::Tcp {
        authority.parse::<http::uri::Authority>()?;
        ensure!(path.starts_with('/'), "Invalid path");
        path.parse::<http::uri::PathAndQuery>()?;
    }
    ensure!(fields.is_empty(), "Unsupported profile parameter");
    Ok(Profile {
        server,
        port,
        uuid: id.into(),
        tls_name,
        fingerprint,
        alpn,
        reality,
        vision,
        encryption: (encryption != "none").then_some(encryption),
        transport,
        authority,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(query: &str) -> Result<Profile> {
        outbound(&format!(
            "vless://00000000-0000-4000-8000-000000000001@[::1]:443?{query}"
        ))
    }

    #[test]
    fn selects_one_worker_from_the_complete_profile() {
        for (query, worker) in [
            ("type=tcp", "vless-stream"),
            ("type=ws&security=tls", "vless-stream"),
            ("type=grpc&serviceName=hello", "vless-http2"),
            ("type=xhttp&mode=stream-one", "vless-http2"),
            ("type=grpc&security=tls&fp=chrome", "vless-xtls"),
            ("security=tls&flow=xtls-rprx-vision", "vless-xtls"),
        ] {
            assert_eq!(parse(query).unwrap().worker(), worker);
        }
        let profile = parse(&format!(
            "type=tcp&security=reality&pbk={}&sid=02ab",
            URL_SAFE_NO_PAD.encode([1; 32])
        ))
        .unwrap();
        assert_eq!(profile.worker(), "vless-xtls");
        assert_eq!(profile.reality.unwrap().short_id, [2, 171]);
    }

    #[test]
    fn invalid_profiles_never_fall_back_to_a_simpler_worker() {
        for query in [
            "security=reality",
            "type=quic",
            "allowInsecure=1",
            "flow=xtls-rprx-vision",
            "type=xhttp&mode=packet-up",
            "type=ws&type=tcp",
            "type=ws&path=%2F%0D%0AHeader",
        ] {
            assert!(parse(query).is_err(), "{query}");
        }
    }
}

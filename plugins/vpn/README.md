# VLESS VPN

VLESS VPN shares one profile among selected Goodwin plugins on Linux. The plugin selects a Rust worker from the link. On first use a short-lived helper downloads just that worker and verifies its size and SHA-256 against the catalog in the installed package. The release server receives the asset name. The profile and its credentials stay on the device.

## Use a profile

1. Sign in to the panel with a password. Open **Plugins** and upload `vpn-linux-aarch64.tar.gz` on the Mini 3.
2. Open **VLESS VPN**, paste a `vless://` link or the provider's HTTP(S) subscription URL and enable the profile. Once the package is installed, profile settings and plugin routing follow the panel's access mode, including open access. A direct link connects to the remote VPN server on the first routed request. For a subscription, Goodwin checks the connection before enabling it.
3. In **Plugins**, turn on **VPN** beside each package that needs it. The choice covers every provider connection in that package. Turn the switch off to use a direct connection.

The OpenAI-compatible WASM provider uses this route for catalog, chat, recognition and speech requests. The native Spotify package uses it for Spotify web requests and keeps its Soloist receiver control connection on the local network. Use a Spotify package that declares native proxy support.

Goodwin remembers both the profile and the assignments across restarts. Replacing a profile closes its old connections. A rejected replacement leaves the previous profile active. Disconnecting or removing the VPN blocks routed requests until a working profile is available or you select **Direct** for that package. The panel never returns the saved link.

### Provider subscriptions

Goodwin fetches the subscription directly so it can recover even when the VPN is down. The URL can return a VLESS link, a newline-separated list, a Base64-encoded list, or a redirect to a link. Goodwin reads up to 128 KiB, tries up to 32 distinct VLESS entries in provider order and selects the first one that passes the tunnel check.

The default interval is 300 seconds. Change it on the VPN page without re-entering the subscription URL. Each check sends a GET through the active tunnel to `https://www.gstatic.com/generate_204`. You can choose another HTTP(S) address in the plugin settings. Use an address that reliably returns HTTP 2xx through your VPN.

As long as checks succeed, Goodwin leaves the subscription server alone. After a failure it fetches the subscription and tests the replacement on a private socket before switching plugin traffic. If the provider or all returned profiles fail, Goodwin keeps the saved profile and retries at the next interval. On restart it first tries the last verified profile. Direct VLESS links have no subscription timer.

Workers are cached under `engines/` in the plugin data directory. Switching back to an already downloaded worker, disconnecting and reconnecting, or restarting Goodwin reuses the verified file offline. A missing, damaged or outdated file is fetched again. During a profile switch the old worker continues serving until its replacement is ready. The helper becomes the selected worker after verification, so it consumes no memory of its own during normal operation.

## Share links

The importer supports a VLESS UUID, host and explicit port, TCP/raw, WebSocket, HTTPUpgrade, HTTP/2, gRPC and XHTTP (stream-one) transports, and plain, TLS or REALITY connections. TCP with TLS or REALITY can use `flow=xtls-rprx-vision`. Common transport parameters include `sni`, `fp`, `alpn`, `pbk`, `sid`, `host`, `path`, `serviceName` and `mode`. Support depends on their combination. The importer reports unsupported options and the engine checks the resulting configuration. The package routes TCP requests, including destination hostnames, through an authenticated local SOCKS endpoint.

| Worker | Profiles |
| --- | --- |
| `vless-stream` | TCP/raw, WebSocket and HTTPUpgrade with plain transport or ordinary TLS |
| `vless-http2` | HTTP/2, gRPC (Gun) and XHTTP stream-one with plain transport or ordinary TLS |
| `vless-xtls` | Profiles using REALITY, Vision, a TLS fingerprint, or VLESS Encryption. These settings take precedence over the underlying transport. |

Goodwin implements the SOCKS endpoint, VLESS request handling, transport selection and data forwarding. The XTLS worker links the TLS, REALITY, Vision framing and encryption libraries from [xray-rust](https://github.com/aimalygin/xray-rust/tree/d481fdfa61afad8663d67e0abdcf80b3e060bfe3). The smaller workers use rustls directly for TLS.

## Build

Run `scripts/build-station.ps1` for the Mini 3 bundle. For a Linux development build, build the panel dependencies and run these commands from the repository root.

```sh
cargo build --locked --bins --profile vpn-release -p goodwin-vpn
cargo build --locked --release --bins --manifest-path plugins/vpn/engines/Cargo.toml --target-dir target
python3 scripts/package-vpn.py target/vpn-release/goodwin-vpn x86_64 target/plugins/vpn-linux-x86_64.tar.gz --engines-dir target/release
```

The packaging script writes the plugin archive and three `vless-*-linux-<arch>.gz` assets beside it. Install only the plugin archive in Goodwin. Tagged builds download workers from that exact GitHub release. For local builds, serve the generated worker files on a web server reachable by the Station and pass its directory URL via `scripts/build-station.ps1 -VpnEngineBaseUrl <url>` or `package-vpn.py --base-url <url>`. The package must point to the assets from its own build.

To build just the stream worker, use `--no-default-features --features stream --bin vless-stream` in place of `--bins`. Use `--features http2 --bin vless-http2` with the same flag for the HTTP/2 worker.

Run the worker tests with `cargo test --locked --manifest-path plugins/vpn/engines/Cargo.toml`. On Linux, `python3 plugins/vpn/tests/interop.py <package.tar.gz> <path-to-xray>` serves the adjacent worker assets locally and exercises cache reuse, repair, cancellation, subscription rotation, and traffic through a local Xray-core v26.11.9 server. This test needs OpenSSL and Python 3.12 or later.

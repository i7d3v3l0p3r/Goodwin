use crate::{
    http2,
    net::{self, Io, Result, Stream, Tunnel},
    types::{Profile, Transport},
    websocket,
};
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use xray_proxy::vless::{DEFAULT_VISION_SEED, VisionStream, VisionStreamIo, VlessResponseStream};
use xray_routing::{Network, Target, TargetAddr};
use xray_transport::{
    BoxedTransportStream, RealityClientConfig, RealityRuntimeEngine, RealityTlsEngine,
    RustlsRealityTlsSessionProvider, TlsClientConfig, TlsConnector,
};
use xray_vless_encryption::{Client, EncryptedStream, Encryption};

enum Security {
    Plain,
    Tls(TlsConnector, TlsClientConfig),
    Reality(RealityRuntimeEngine, RealityClientConfig, Target),
}

pub struct Dialer {
    profile: Profile,
    security: Security,
    encryption: Option<Client>,
    uuid: [u8; 16],
}

impl Dialer {
    pub fn new(profile: Profile) -> Result<Self> {
        let alpn = net::protocols(&profile)?;
        let security = if let Some(reality) = &profile.reality {
            let cfg = RealityClientConfig {
                server_name: profile.tls_name.clone().ok_or("Missing REALITY name")?,
                fingerprint: profile.fingerprint.clone().ok_or("Missing fingerprint")?,
                public_key: reality.key,
                short_id: reality.short_id.clone(),
                spider_x: "/".into(),
                mldsa65_verify: None,
            };
            let addr = profile
                .server
                .parse()
                .map(TargetAddr::Ip)
                .unwrap_or_else(|_| TargetAddr::Domain(profile.server.clone()));
            let target = Target::new(addr, profile.port, Network::Tcp);
            let engine = RealityRuntimeEngine::new(Arc::new(RustlsRealityTlsSessionProvider));
            engine
                .prepare_preconnected(&cfg, &target)?
                .ok_or("Unsupported REALITY handshake")?;
            Security::Reality(engine, cfg, target)
        } else if let Some(name) = &profile.tls_name {
            let cfg = TlsClientConfig {
                server_name: name.clone(),
                allow_insecure: false,
                pinned_peer_cert_sha256: vec![],
                verify_peer_cert_by_name: vec![],
                alpn,
                fingerprint: profile.fingerprint.clone(),
            };
            let tls = TlsConnector::system()?;
            tls.client_config_for(&cfg)?;
            Security::Tls(tls, cfg)
        } else {
            Security::Plain
        };
        let encryption = profile
            .encryption
            .as_deref()
            .map(str::parse::<Encryption>)
            .transpose()?
            .and_then(|cfg| cfg.client().cloned())
            .map(Client::new);
        let uuid = *uuid::Uuid::parse_str(&profile.uuid)?.as_bytes();
        Ok(Self {
            profile,
            security,
            encryption,
            uuid,
        })
    }
    pub async fn open(&self, header: &[u8]) -> Result<Tunnel> {
        let socket = net::tcp(&self.profile).await?;
        let mut secure: BoxedTransportStream = match &self.security {
            Security::Plain => Box::new(socket),
            Security::Tls(tls, config) => tls.connect_stream(Box::new(socket), config).await?,
            Security::Reality(engine, config, target) => {
                engine
                    .prepare_preconnected(config, target)?
                    .ok_or("Missing REALITY session")?
                    .complete(socket)
                    .await?
            }
        };
        if self.profile.vision {
            let mut carrier = if let Some(cipher) = &self.encryption {
                secure.release_record_alignment();
                VisionCarrier::Encrypted(Box::new(cipher.connect(secure).await?))
            } else {
                VisionCarrier::Tls(secure)
            };
            carrier.write_all(header).await?;
            carrier.flush().await?;
            let stream = VisionStream::new(
                VlessResponseStream::new(carrier),
                self.uuid,
                DEFAULT_VISION_SEED,
            );
            Ok(Tunnel {
                stream: Box::new(stream),
                read_header: false,
            })
        } else {
            secure.release_record_alignment();
            let socket = Box::new(secure);
            let mut stream: Stream = match self.profile.transport {
                Transport::Tcp | Transport::WebSocket | Transport::HttpUpgrade => {
                    websocket::connect(&self.profile, socket).await?
                }
                _ => http2::connect(&self.profile, socket).await?,
            };
            if let Some(cipher) = &self.encryption {
                stream = Box::new(cipher.connect(stream).await?);
            }
            Tunnel::start(stream, header).await
        }
    }
}

// Vision's direct records bypass the outer record codec. Keep ownership of that
// exact connection, including any bytes already buffered during the handshake.
enum VisionCarrier {
    Tls(BoxedTransportStream),
    Encrypted(Box<EncryptedStream<BoxedTransportStream>>),
}
impl VisionCarrier {
    fn io(&mut self) -> &mut dyn Io {
        match self {
            Self::Tls(s) => s,
            Self::Encrypted(s) => s,
        }
    }
}
impl AsyncRead for VisionCarrier {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_read(cx, b)
    }
}
impl AsyncWrite for VisionCarrier {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.io()).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_shutdown(cx)
    }
}
impl VisionStreamIo for VisionCarrier {
    fn release_record_alignment(&mut self) {
        if let Self::Tls(s) = self {
            s.release_record_alignment();
        }
    }
    fn poll_read_direct(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(s) => Pin::new(&mut **s).poll_read_direct(cx, b),
            Self::Encrypted(s) => s.poll_read_vision_direct(cx, b),
        }
    }
    fn poll_write_direct(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tls(s) => Pin::new(&mut **s).poll_write_direct(cx, b),
            Self::Encrypted(s) => s.poll_write_vision_direct(cx, b),
        }
    }
    fn poll_flush_direct(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(s) => Pin::new(&mut **s).poll_flush_direct(cx),
            Self::Encrypted(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown_direct(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(s) => Pin::new(&mut **s).poll_shutdown_direct(cx),
            Self::Encrypted(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

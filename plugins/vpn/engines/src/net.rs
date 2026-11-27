use crate::types::{Profile, Transport};
use std::{
    error::Error,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    net::TcpStream,
    task::JoinHandle,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}
pub type Stream = Box<dyn Io>;

pub async fn tcp(profile: &Profile) -> Result<TcpStream> {
    let socket = TcpStream::connect((profile.server.as_str(), profile.port)).await?;
    socket.set_nodelay(true)?;
    Ok(socket)
}

pub fn protocols(profile: &Profile) -> Result<Vec<String>> {
    let expected = match profile.transport {
        Transport::WebSocket | Transport::HttpUpgrade => "http/1.1",
        Transport::Http2 | Transport::Grpc | Transport::Xhttp => "h2",
        Transport::Tcp => return Ok(profile.alpn.clone()),
    };
    if !profile.alpn.is_empty() && !profile.alpn.iter().any(|p| p == expected) {
        return Err("ALPN conflicts with transport".into());
    }
    Ok(vec![expected.into()])
}

pub struct Dialer(Option<(TlsConnector, ServerName<'static>)>);
impl Dialer {
    pub fn new(profile: &Profile) -> Result<Self> {
        let Some(name) = &profile.tls_name else {
            return Ok(Self(None));
        };
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = protocols(profile)?
            .into_iter()
            .map(String::into_bytes)
            .collect();
        Ok(Self(Some((
            TlsConnector::from(Arc::new(config)),
            ServerName::try_from(name.clone())?,
        ))))
    }
    pub async fn connect(&self, profile: &Profile) -> Result<Stream> {
        let socket = tcp(profile).await?;
        match &self.0 {
            None => Ok(Box::new(socket)),
            Some((tls, name)) => Ok(Box::new(tls.connect(name.clone(), socket).await?)),
        }
    }
}

pub struct Tunnel {
    pub stream: Stream,
    pub read_header: bool,
}
impl Tunnel {
    pub async fn start(mut stream: Stream, header: &[u8]) -> Result<Self> {
        stream.write_all(header).await?;
        stream.flush().await?;
        Ok(Self {
            stream,
            read_header: true,
        })
    }
}

// Dropping a cancelled SOCKS request must also cancel the transport tasks and close their sockets.
pub struct Bridge {
    pub pipe: DuplexStream,
    pub tasks: Vec<JoinHandle<()>>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl AsyncRead for Bridge {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_read(cx, b)
    }
}
impl AsyncWrite for Bridge {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.pipe).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_shutdown(cx)
    }
}

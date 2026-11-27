use crate::{
    net::{Result, Tunnel},
    types::{Launch, Profile},
};
use std::{
    future::Future,
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    task::JoinSet,
    time::timeout,
};
use uuid::Uuid;

const HANDSHAKE: Duration = Duration::from_secs(12);

pub async fn serve<F, Fut>(name: &str, init: impl FnOnce(Profile) -> Result<F>) -> Result<()>
where
    F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Tunnel>> + Send,
{
    let path = PathBuf::from(std::env::args_os().nth(1).ok_or("Missing socket")?);
    let launch: Launch = serde_json::from_reader(std::io::stdin().lock().take(32768))?;
    if launch.profile.worker() != name || launch.password.is_empty() || launch.password.len() > 255
    {
        return Err("Invalid launch configuration".into());
    }
    let mut header = vec![0];
    header.extend_from_slice(Uuid::parse_str(&launch.profile.uuid)?.as_bytes());
    if launch.profile.vision {
        let flow = b"xtls-rprx-vision";
        header.extend_from_slice(&[2 + flow.len() as u8, 10, flow.len() as u8]);
        header.extend_from_slice(flow);
    } else {
        header.push(0);
    }
    header.push(1); // TCP CONNECT
    let open = init(launch.profile)?;
    let shared = Arc::new((header, launch.password, open));
    let listener = UnixListener::bind(path)?;
    std::io::stdout().lock().write_all(&[1])?;
    std::io::stdout().lock().flush()?;
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            Some(_) = tasks.join_next() => {},
            incoming = listener.accept(), if tasks.len() < 64 => {
                let (mut client, _) = incoming?;
                let shared = shared.clone();
                tasks.spawn(async move {
                    let (header,password,open) = &*shared;
                    let address = timeout(HANDSHAKE, negotiate(&mut client,password)).await??;
                    let mut request = header.clone();
                    request.extend(address);
                    let tunnel = match timeout(HANDSHAKE,open(request)).await {
                        Ok(Ok(tunnel)) => tunnel,
                        _ => { reply(&mut client,1).await?; return Ok(()); }
                    };
                    reply(&mut client,0).await?;
                    relay(client,tunnel).await
                });
            }
        }
    }
}

async fn reply(stream: &mut UnixStream, code: u8) -> Result<()> {
    stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    Ok(())
}

async fn field(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let len = stream.read_u8().await?;
    let mut data = vec![0; len as usize];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

async fn negotiate(stream: &mut UnixStream, password: &str) -> Result<Vec<u8>> {
    if stream.read_u8().await? != 5 {
        return Err("Invalid SOCKS version".into());
    }
    if !field(stream).await?.contains(&2) {
        stream.write_all(&[5, 255]).await?;
        return Err("Authentication required".into());
    }
    stream.write_all(&[5, 2]).await?;
    if stream.read_u8().await? != 1
        || field(stream).await? != b"goodwin"
        || field(stream).await? != password.as_bytes()
    {
        stream.write_all(&[1, 1]).await?;
        return Err("Authentication failed".into());
    }
    stream.write_all(&[1, 0]).await?;
    let mut command = [0; 4];
    stream.read_exact(&mut command).await?;
    if command[..3] != [5, 1, 0] {
        reply(stream, 7).await?;
        return Err("Unsupported SOCKS command".into());
    }
    let (kind, len) = match command[3] {
        1 => (1, 4),
        4 => (3, 16),
        3 => (2, stream.read_u8().await? as usize),
        _ => {
            reply(stream, 8).await?;
            return Err("Invalid address type".into());
        }
    };
    if len == 0 {
        return Err("Empty destination".into());
    }
    let mut address = vec![0, 0, kind];
    if kind == 2 {
        address.push(len as u8);
    }
    let offset = address.len();
    address.resize(offset + len, 0);
    stream.read_exact(&mut address[offset..]).await?;
    stream.read_exact(&mut address[..2]).await?;
    Ok(address)
}

async fn response(stream: &mut (impl AsyncRead + Unpin)) -> Result<()> {
    if stream.read_u8().await? != 0 {
        return Err("Invalid VLESS response version".into());
    }
    let size = stream.read_u8().await? as usize;
    stream.read_exact(&mut [0u8; 255][..size]).await?;
    Ok(())
}

async fn relay(client: UnixStream, tunnel: Tunnel) -> Result<()> {
    let (mut source, mut sink) = client.into_split();
    let (mut read, mut write) = tokio::io::split(tunnel.stream);
    let upload = async {
        tokio::io::copy(&mut source, &mut write).await?;
        write.shutdown().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    };
    let download = async {
        if tunnel.read_header {
            timeout(Duration::from_secs(60), response(&mut read)).await??;
        }
        tokio::io::copy(&mut read, &mut sink).await?;
        sink.shutdown().await?;
        Ok(())
    };
    tokio::pin!(upload, download);
    tokio::select! {
        r = &mut download => r,
        r = &mut upload => {r?; download.await}
    }
}

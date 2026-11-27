use crate::{
    net::{Bridge, Result, Stream},
    types::{Profile, Transport},
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{
    client_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

pub async fn connect(profile: &Profile, mut socket: Stream) -> Result<Stream> {
    match profile.transport {
        Transport::Tcp => Ok(socket),
        Transport::HttpUpgrade => {
            let request = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
                profile.path, profile.authority
            );
            socket.write_all(request.as_bytes()).await?;
            socket.flush().await?;
            // Leave the first tunneled byte in the stream even if the response and payload coalesce.
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                if head.len() >= 16 * 1024 {
                    return Err("Upgrade headers too large".into());
                }
                head.push(socket.read_u8().await?);
            }
            let mut lines = std::str::from_utf8(&head)?.split("\r\n");
            let status = lines.next().unwrap_or_default();
            let mut parts = status.split_whitespace();
            if parts.next() != Some("HTTP/1.1") || parts.next() != Some("101") {
                return Err("HTTP upgrade refused".into());
            }
            let mut upgrade = false;
            let mut connection = false;
            for (k, v) in lines.filter_map(|line| line.split_once(':')) {
                if k.eq_ignore_ascii_case("upgrade") {
                    upgrade |= v.trim().eq_ignore_ascii_case("websocket");
                }
                if k.eq_ignore_ascii_case("connection") {
                    connection |= v
                        .split(',')
                        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"));
                }
            }
            if !upgrade || !connection {
                return Err("Invalid upgrade response".into());
            }
            Ok(socket)
        }
        Transport::WebSocket => {
            let uri = format!("ws://{}{}", profile.authority, profile.path);
            let config = WebSocketConfig::default()
                .read_buffer_size(8192)
                .write_buffer_size(8192)
                .max_message_size(Some(4 * 1024 * 1024))
                .max_frame_size(Some(4 * 1024 * 1024));
            let (ws, _) = client_async_with_config(uri, socket, Some(config)).await?;
            let (mut tx, mut rx) = ws.split();
            let (pipe, inner) = tokio::io::duplex(8192);
            let (mut input, mut output) = tokio::io::split(inner);
            let task = tokio::spawn(async move {
                let upload = async {
                    let mut data = [0; 8192];
                    loop {
                        let n = input.read(&mut data).await?;
                        if n == 0 {
                            break;
                        }
                        tx.send(Message::Binary(Bytes::copy_from_slice(&data[..n])))
                            .await?;
                    }
                    tx.close().await?;
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
                };
                let download = async {
                    while let Some(msg) = rx.next().await {
                        match msg? {
                            Message::Binary(data) => output.write_all(&data).await?,
                            Message::Close(_) => break,
                            Message::Ping(_) | Message::Pong(_) => {}
                            _ => return Err("Unexpected WebSocket message".into()),
                        }
                    }
                    output.shutdown().await?;
                    Ok(())
                };
                let _ = tokio::try_join!(upload, download);
            });
            Ok(Box::new(Bridge {
                pipe,
                tasks: vec![task],
            }))
        }
        _ => Err("Unsupported stream transport".into()),
    }
}

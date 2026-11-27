use crate::{
    net::{Bridge, Result, Stream},
    types::{Profile, Transport},
};
use bytes::{Buf, Bytes, BytesMut};
use futures_util::future::poll_fn;
use h2::SendStream;
use http::{Method, Request, Version};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Message)]
struct Hunk {
    #[prost(bytes = "bytes", tag = "1")]
    data: Bytes,
}

fn grpc_frame(data: Bytes) -> Bytes {
    let hunk = Hunk { data };
    let len = hunk.encoded_len();
    let mut frame = Vec::with_capacity(5 + len);
    frame.push(0);
    frame.extend_from_slice(&(len as u32).to_be_bytes());
    frame.extend(hunk.encode_to_vec());
    frame.into()
}

fn grpc_message(buffer: &mut BytesMut) -> Result<Option<Bytes>> {
    if buffer.len() < 5 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buffer[1..5].try_into()?) as usize;
    if buffer[0] != 0 || len > 4 * 1024 * 1024 {
        return Err("Invalid gRPC frame".into());
    }
    if buffer.len() < 5 + len {
        return Ok(None);
    }
    buffer.advance(5);
    Ok(Some(Hunk::decode(buffer.split_to(len).freeze())?.data))
}

async fn send(tx: &mut SendStream<Bytes>, mut bytes: Bytes) -> Result<()> {
    while !bytes.is_empty() {
        tx.reserve_capacity(bytes.len());
        let capacity = poll_fn(|cx| tx.poll_capacity(cx))
            .await
            .ok_or("Closed HTTP/2 stream")??;
        let count = capacity.min(bytes.len());
        if count > 0 {
            tx.send_data(bytes.split_to(count), false)?;
        }
    }
    Ok(())
}

pub async fn connect(profile: &Profile, socket: Stream) -> Result<Stream> {
    let grpc = profile.transport == Transport::Grpc;
    let xhttp = profile.transport == Transport::Xhttp;
    if !grpc && !xhttp && profile.transport != Transport::Http2 {
        return Err("Invalid HTTP/2 transport".into());
    }
    let scheme = if profile.tls_name.is_some() {
        "https"
    } else {
        "http"
    };
    let origin = format!("{scheme}://{}", profile.authority);
    let (path, query) = profile.path.split_once('?').unwrap_or((&profile.path, ""));
    let slash = if xhttp && !path.ends_with('/') {
        "/"
    } else {
        ""
    };
    let mut uri = format!("{origin}{path}{slash}");
    let mut req = Request::builder()
        .method(Method::POST)
        .version(Version::HTTP_2);
    if xhttp {
        let random = uuid::Uuid::new_v4();
        let size = 100
            + usize::from(u16::from_be_bytes([
                random.as_bytes()[0],
                random.as_bytes()[1],
            ])) % 901;
        req = req.header("referer", format!("{uri}?x_padding={}", "X".repeat(size)));
    }
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(query);
    }
    if xhttp || grpc {
        req = req.header("content-type", "application/grpc");
    }
    if grpc {
        req = req.header("te", "trailers");
    }
    let request = req.uri(uri).body(())?;
    let (client, connection) = h2::client::handshake(socket).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let (pipe, inner) = tokio::io::duplex(8192);
    let mut bridge = Bridge {
        pipe,
        tasks: vec![driver],
    };
    let (response, mut tx) = client.ready().await?.send_request(request, false)?;
    let (mut input, mut output) = tokio::io::split(inner);
    bridge.tasks.push(tokio::spawn(async move {
        let upload = async {
            let mut buf = [0; 8192];
            loop {
                let n = input.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                let bytes = Bytes::copy_from_slice(&buf[..n]);
                send(&mut tx, if grpc { grpc_frame(bytes) } else { bytes }).await?;
            }
            tx.send_data(Bytes::new(), true)?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        };
        let download = async {
            let response = response.await?;
            if response.status() != 200 {
                return Err("HTTP/2 tunnel refused".into());
            }
            let mut body = response.into_body();
            let mut pending = BytesMut::new();
            while let Some(data) = body.data().await {
                let data = data?;
                let n = data.len();
                if grpc {
                    pending.extend_from_slice(&data);
                    while let Some(payload) = grpc_message(&mut pending)? {
                        output.write_all(&payload).await?;
                    }
                } else {
                    output.write_all(&data).await?;
                }
                body.flow_control().release_capacity(n)?;
            }
            if !pending.is_empty() {
                return Err("Incomplete gRPC message".into());
            }
            output.shutdown().await?;
            Ok(())
        };
        let _ = tokio::try_join!(upload, download);
    }));
    Ok(Box::new(bridge))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_http2_can_send_before_the_response_headers_arrive() {
        let profile = Profile {
            server: "localhost".into(),
            port: 80,
            uuid: String::new(),
            tls_name: None,
            fingerprint: None,
            alpn: vec![],
            reality: None,
            vision: false,
            encryption: None,
            transport: Transport::Http2,
            authority: "voice.test".into(),
            path: "/tunnel".into(),
        };
        let (client, server) = tokio::io::duplex(8192);
        let peer = tokio::spawn(async move {
            let mut connection = h2::server::handshake(server).await.unwrap();
            let (request, mut reply) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.method(), Method::POST);
            assert_eq!(request.uri().authority().unwrap(), "voice.test");
            assert_eq!(request.uri().path(), "/tunnel");
            let driver = tokio::spawn(async move { while connection.accept().await.is_some() {} });
            let payload = request.into_body().data().await.unwrap().unwrap();
            assert_eq!(payload, "request");
            let mut tx = reply.send_response(http::Response::new(()), false).unwrap();
            tx.send_data(Bytes::from_static(b"response"), true).unwrap();
            driver.await.unwrap();
        });
        let mut stream = connect(&profile, Box::new(client)).await.unwrap();
        stream.write_all(b"request").await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response, b"response");
        drop(stream);
        peer.await.unwrap();
    }

    #[test]
    fn decodes_fragmented_and_consecutive_grpc_messages() {
        let frame = grpc_frame(Bytes::from_static(b"goodwin"));
        let mut buf = BytesMut::from(&frame[..4]);
        assert!(grpc_message(&mut buf).unwrap().is_none());
        buf.extend_from_slice(&frame[4..]);
        buf.extend_from_slice(&frame);
        for _ in 0..2 {
            assert_eq!(grpc_message(&mut buf).unwrap().unwrap(), &b"goodwin"[..]);
        }
        assert!(buf.is_empty());
        assert!(grpc_message(&mut BytesMut::from(&[0, 255, 255, 255, 255][..])).is_err());
    }
}

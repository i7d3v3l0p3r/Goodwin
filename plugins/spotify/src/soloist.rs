use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use url::Url;

use crate::Outcome;

pub(crate) fn address(value: &str) -> Outcome<Url> {
    let value = value.trim();
    let url = Url::parse(if value.is_empty() {
        "ws://127.0.0.1:9090"
    } else {
        value
    })
    .map_err(|_| "invalid_soloist_address")?;
    if !matches!(url.scheme(), "ws" | "wss")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_soloist_address");
    }
    Ok(url)
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) async fn connect(url: &Url) -> Outcome<Socket> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(512 * 1024))
        .max_frame_size(Some(512 * 1024));
    connect_async_with_config(url.as_str(), Some(config), true)
        .await
        .map(|(socket, _)| socket)
        .map_err(|_| "soloist_unreachable")
}

pub(super) async fn send(socket: &mut Socket, request: &Value) -> Outcome<()> {
    socket
        .send(Message::text(request.to_string()))
        .await
        .map_err(|_| "soloist_disconnected")
}

pub(super) async fn event(socket: &mut Socket) -> Outcome<Value> {
    while let Some(frame) = socket.next().await {
        let frame = frame.map_err(|_| "soloist_disconnected")?;
        if frame.is_close() {
            break;
        }
        if frame.is_text() {
            return serde_json::from_slice(&frame.into_data())
                .map_err(|_| "invalid_soloist_response");
        }
    }
    Err("soloist_disconnected")
}

pub(crate) async fn command(url: &Url, name: &str, uri: Option<&str>) -> Outcome<()> {
    let mut socket = connect(url).await?;
    let mut request = json!({"type": "command", "command": name});
    if let Some(uri) = uri {
        request["uri"] = Value::String(uri.into());
    }
    let mut sent = false;
    loop {
        let event = event(&mut socket).await?;
        match event["type"].as_str() {
            Some("auth_state") if event["logged_in"] == false => {
                return Err("select_soloist_in_spotify");
            }
            Some("auth_state") if event["logged_in"] == true && !sent => {
                send(&mut socket, &request).await?;
                sent = true;
            }
            Some("command_result") if sent && event["command"] == name => return Ok(()),
            Some("error") => return Err("soloist_rejected_command"),
            _ => {}
        }
    }
}

use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_openai_compatible::{Config, Protocol, Upstream};
use goodwin_plugin::provider::{ChatMessage, ChatRequest, ChatRole, SpeechRequest};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

#[path = "../../../src/test_support.rs"]
mod test_support;

async fn request(socket: &mut TcpStream) -> (String, Value) {
    let mut headers = Vec::new();
    let mut byte = [0];
    while !headers.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).await.unwrap();
        headers.push(byte[0]);
    }
    let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
    let length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    (headers, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn speech_emits_complete_pcm_frames_before_the_http_body_ends() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        speech_model: "fixture".into(),
        ..Config::default()
    };
    let (first_audio, arrived) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (headers, body) = request(&mut socket).await;
        assert!(headers.starts_with("post /v1/audio/speech "));
        assert!(headers.contains("\r\naccept: audio/pcm\r\n"));
        assert_eq!(body["input"], "Готово");
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm;rate=16000;channels=2\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n\x01\x02\x03\x04\x05\r\n").await.unwrap();
        timeout(Duration::from_secs(5), arrived)
            .await
            .unwrap()
            .unwrap();
        socket
            .write_all(b"3\r\n\x06\x07\x08\r\n0\r\n\r\n")
            .await
            .unwrap();
    });
    let audio = tokio::task::spawn_blocking(move || {
        let client = Upstream::new(Protocol::OpenAi).unwrap();
        let mut first_audio = Some(first_audio);
        let mut audio = Vec::new();
        client
            .synthesize_stream(
                &config,
                SpeechRequest {
                    text: "Готово".into(),
                    selection: Default::default(),
                },
                &mut |chunk| {
                    assert_eq!((chunk.sample_rate, chunk.channels), (16_000, 2));
                    let pcm = STANDARD.decode(chunk.pcm_base64).unwrap();
                    assert!(pcm.len().is_multiple_of(4));
                    audio.extend(pcm);
                    if let Some(first) = first_audio.take() {
                        first.send(()).unwrap();
                    }
                    Ok(())
                },
            )
            .unwrap();
        audio
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(audio, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[tokio::test]
async fn chat_emits_visible_text_before_the_server_finishes_and_reports_a_stream_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        chat_model: "fixture".into(),
        ..Config::default()
    };
    let (first_text, arrived) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (headers, body) = request(&mut socket).await;
        assert!(headers.starts_with("post /v1/chat/completions "));
        assert!(headers.contains("\r\naccept: text/event-stream\r\n"));
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "fixture");
        assert_eq!(body["messages"][0]["content"], "Привет");

        let first = concat!(
            ": processing\r\n\r\n",
            "data: {\"choices\":[{\"index\":0,\r\n",
            "data: \"delta\":{\"content\":\"Привет\",\"reasoning\":\"private\"}}]}\r\n\r\n",
        );
        let terminal = "data: {\"error\":{\"message\":\"private server detail\"}}\n\n";
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
            first.len() + terminal.len()
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.write_all(first.as_bytes()).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(5), arrived)
                .await
                .unwrap()
                .unwrap(),
            "Привет"
        );
        socket.write_all(terminal.as_bytes()).await.unwrap();
    });
    tokio::task::spawn_blocking(move || {
        let mut first_text = Some(first_text);
        let client = Upstream::new(Protocol::OpenAi).unwrap();
        let error = client
            .complete_stream(
                &config,
                ChatRequest {
                    messages: vec![ChatMessage::text(ChatRole::User, "Привет")],
                    tools: Vec::new(),
                    tool_choice: None,
                    max_completion_tokens: 256,
                    selection: Default::default(),
                },
                &mut |chunk| {
                    first_text.take().unwrap().send(chunk.to_owned()).unwrap();
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "Provider chat stream returned an error");
    })
    .await
    .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn chat_retries_without_streaming_when_a_compatible_server_rejects_it() {
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};

    let (url, server) = test_support::serve(Router::new().route(
        "/v1/chat/completions",
        post(|Json(body): Json<Value>| async move {
            if body["stream"] == true {
                StatusCode::NOT_IMPLEMENTED.into_response()
            } else {
                Json(json!({"choices": [{"message": {"role": "assistant", "content": "Готово"}}]}))
                    .into_response()
            }
        }),
    ))
    .await;
    tokio::task::spawn_blocking(move || {
        let config = Config {
            base_url: format!("{url}/v1"),
            chat_model: "fixture".into(),
            ..Config::default()
        };
        let mut text = String::new();
        Upstream::new(Protocol::OpenAi)
            .unwrap()
            .complete_stream(
                &config,
                ChatRequest {
                    messages: vec![ChatMessage::text(ChatRole::User, "Привет")],
                    tools: Vec::new(),
                    tool_choice: None,
                    max_completion_tokens: 256,
                    selection: Default::default(),
                },
                &mut |chunk| {
                    text.push_str(chunk);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(text, "Готово");
    })
    .await
    .unwrap();
    server.abort();
}

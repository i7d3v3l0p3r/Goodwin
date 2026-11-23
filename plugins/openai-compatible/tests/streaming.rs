use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_openai_compatible::{Config, Protocol, Upstream};
use goodwin_plugin::provider::SpeechRequest;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    time::timeout,
};

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
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
        assert!(request.starts_with("post /v1/audio/speech "));
        assert!(request.contains("\r\naccept: audio/pcm\r\n"));
        let length = request
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        socket.read_exact(&mut vec![0; length]).await.unwrap();
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

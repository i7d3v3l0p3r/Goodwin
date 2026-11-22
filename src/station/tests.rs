use std::time::Duration;

use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;
use serde_json::{Value, json};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    time::{sleep, timeout},
};

use crate::{
    app::{App, Source, Stage},
    config::{Access, Config, Settings},
    openrouter::OpenRouter,
    panel,
    test_support::serve,
};

use super::{
    MicrophoneOptions, Player,
    playback::TestSink,
    protocol::{Framing, write_samples},
    run_microphone,
};

async fn wait_for(mut check: impl FnMut() -> bool) {
    timeout(Duration::from_secs(8), async {
        while !check() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected runtime state");
}

#[tokio::test]
async fn voice_request_through_microphone_panel_and_openrouter() {
    let provider = Router::new()
        .route(
            "/audio/transcriptions",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer mock-key");
                assert_eq!(body["language"], "ru");
                let wav = STANDARD
                    .decode(body["input_audio"]["data"].as_str().unwrap())
                    .unwrap();
                assert_eq!(&wav[..4], b"RIFF");
                assert!(
                    wav[44..]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .any(|bytes| *bytes == 16_384_i16.to_le_bytes())
                );
                Json(json!({ "text": "Сколько будет два плюс два?" }))
            }),
        )
        .route(
            "/chat/completions",
            post(|Json(body): Json<Value>| async move {
                let messages = body["messages"].as_array().unwrap();
                let text = messages.last().unwrap()["content"].as_str().unwrap();
                if text == "Fail the cloud" {
                    assert_eq!(messages[2]["content"], "Четыре");
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error": "service unavailable"})),
                    );
                }
                assert_eq!(text, "Сколько будет два плюс два?");
                (
                    StatusCode::OK,
                    Json(json!({ "choices": [{ "message": { "content": "Четыре" } }] })),
                )
            }),
        )
        .route(
            "/audio/speech",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["input"], "Четыре");
                assert_eq!(body["response_format"], "pcm");
                (
                    [(header::CONTENT_TYPE, "audio/pcm; rate=16000; channels=1")],
                    vec![1_u8, 2, 3, 4],
                )
            }),
        );
    let (cloud, cloud_task) = serve(provider).await;

    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        access: Access::Open,
        settings: Settings {
            listening: false,
            ..Settings::default()
        },
        key: "mock-key".into(),
    };
    let sink = TestSink::default();
    let app = App::new(
        dir.path().into(),
        config,
        OpenRouter::new().unwrap().pointed_at(cloud),
        Player::recording_to(sink.clone()),
    );
    let (panel, panel_task) = serve(panel::router(app.clone())).await;
    let mic = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = mic.local_addr().unwrap();
    let (start_audio, start_event) = tokio::sync::oneshot::channel();
    let mic_task = tokio::spawn(async move {
        let (mut audio, _) = mic.accept().await.unwrap();
        let length = audio.read_u32_le().await.unwrap();
        let mut request = vec![0; length as usize];
        audio.read_exact(&mut request).await.unwrap();
        assert_eq!(request, Framing::Typed.subscription().encode_to_vec());
        write_samples(&mut audio, 0, 1).await;
        start_event.await.unwrap();
        write_samples(&mut audio, 1200, 12).await;
        write_samples(&mut audio, 0, 36).await;
        std::future::pending::<()>().await;
    });
    let capture = tokio::spawn(run_microphone(
        app.clone(),
        MicrophoneOptions { address, gain: 1.0 },
    ));
    let client = reqwest::Client::new();

    timeout(Duration::from_secs(8), async {
        loop {
            let state: Value = client
                .get(format!("{panel}/api/state"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if state["microphone"] == true {
                assert_eq!(state["model"], false);
                assert_eq!(state["settings"]["listening"], false);
                assert!(state.get("key").is_none());
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let status = client
        .post(format!("{panel}/api/test"))
        .json(&json!({"mode": "manual"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::ACCEPTED);
    let status = client
        .post(format!("{panel}/api/chat"))
        .json(&json!({"message": "Concurrent"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::CONFLICT);
    start_audio.send(()).unwrap();
    wait_for(|| app.active().is_none()).await;
    {
        let audio = sink.lock().unwrap();
        assert_eq!(audio.len(), 1);
        assert_eq!(&audio[0][44..], &[1, 2, 3, 4]);
    }

    let state: Value = client
        .get(format!("{panel}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["messages"][1]["text"], "Четыре");
    assert_eq!(state["last"]["error"], Value::Null);
    let stages: Vec<_> = state["last"]["timings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|timing| timing["stage"].as_str().unwrap())
        .collect();
    assert_eq!(
        stages,
        [
            "recording",
            "transcribing",
            "thinking",
            "synthesizing",
            "playing"
        ]
    );

    let status = client
        .post(format!("{panel}/api/chat"))
        .json(&json!({"message": "Fail the cloud"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::ACCEPTED);
    wait_for(|| app.active().is_none()).await;
    let state: Value = client
        .get(format!("{panel}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["last"]["error"], "cloud");
    assert_eq!(sink.lock().unwrap().len(), 1);

    for task in [capture, mic_task, panel_task, cloud_task] {
        task.abort();
    }
}

#[tokio::test]
async fn wake_word_without_a_transcribed_question_plays_a_local_retry_prompt() {
    let (cloud, cloud_task) = serve(Router::new().route(
        "/audio/transcriptions",
        post(|| async { Json(json!({"text": "Гудвин."})) }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let sink = TestSink::default();
    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            key: "mock-key".into(),
        },
        OpenRouter::new().unwrap().pointed_at(cloud),
        Player::recording_to(sink.clone()),
    );
    app.set_microphone(true);
    app.set_model(true);
    let id = app.start(Source::Wake).ok().expect("start wake request");
    app.stage(id, Stage::Transcribing);
    app.voice(id, vec![128; 320], true).await;

    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], "no_speech");
    assert_eq!(state["messages"], json!([]));
    let played = sink.lock().unwrap();
    assert_eq!(played.len(), 1);
    assert_eq!(played[0], include_bytes!("../../assets/no-speech-ru.wav"));
    cloud_task.abort();
}

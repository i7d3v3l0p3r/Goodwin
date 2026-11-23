use std::time::Duration;

use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::provider::ModelSelection;
use prost::Message;
use serde_json::{Value, json};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    time::{sleep, timeout},
};

use crate::{
    app::{App, Source, Stage},
    assistant::{AssistantSettings, PipelineSettings, StageSettings},
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    panel,
    plugins::Process,
    test_support::serve,
};

#[tokio::test]
async fn the_pipeline_passes_the_selected_models_and_voice_to_its_provider() {
    let dir = tempfile::tempdir().unwrap();
    let sink = TestSink::default();
    let plugin_id = "fixture-provider".to_owned();
    let stage = |model: &str, parameters| StageSettings {
        provider: plugin_id.clone(),
        selection: ModelSelection {
            model: Some(model.into()),
            parameters,
        },
    };
    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings {
                assistant: AssistantSettings {
                    pipeline: PipelineSettings {
                        speech_streaming: false,
                        transcription: stage("fixture/stt", Default::default()),
                        chat: stage("fixture/chat", Default::default()),
                        speech: stage(
                            "fixture/tts",
                            json!({"voice":"Russian","speed":1.1})
                                .as_object()
                                .unwrap()
                                .clone(),
                        ),
                    },
                    ..AssistantSettings::default()
                },
                ..Settings::default()
            },
            system_prompt: String::new(),
        },
        Player::recording_to(sink.clone()),
        DEFAULT_MICROPHONE_GAIN,
    );
    let pcm = vec![0xab; 128 * 1024];
    let encoded = STANDARD.encode(&pcm);
    app.plugins.mock_provider(
        &plugin_id,
        Process::mock_rpc(move |method, body| {
            Ok(match method {
                "provider.transcribe" => {
                    assert_eq!(body["selection"]["model"], "fixture/stt");
                    assert_eq!(body["language"], "ru");
                    let wav = STANDARD
                        .decode(body["wavBase64"].as_str().unwrap())
                        .unwrap();
                    assert_eq!(&wav[..4], b"RIFF");
                    json!({"text": "Сколько будет два плюс два?"})
                }
                "provider.complete" => {
                    assert_eq!(body["selection"]["model"], "fixture/chat");
                    assert!(body.get("provider").is_none());
                    let messages = body["messages"].as_array().unwrap();
                    assert_eq!(messages[0]["role"], "system");
                    assert_eq!(
                        messages.last().unwrap()["content"],
                        "Сколько будет два плюс два?"
                    );
                    json!({"message": {"role": "assistant", "content": "Четыре"}})
                }
                "provider.synthesize" => {
                    assert_eq!(body, json!({
                        "text": "Четыре",
                        "selection": {"model":"fixture/tts", "parameters":{"voice":"Russian","speed":1.1}}
                    }));
                    json!({"pcmBase64": encoded, "sampleRate": 24_000, "channels": 1})
                }
                _ => panic!("Unexpected method {method}"),
            })
        }),
    );
    app.set_microphone(true);
    let id = app.start(Source::ManualTest).ok().unwrap();
    app.voice(id, vec![1000; 1024], false, RecordingGain::Automatic(4.0))
        .await;
    assert_eq!(sink.lock().unwrap()[0][44..], pcm);
    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], Value::Null);
    assert_eq!(
        state["plugins"][0]["providers"],
        json!(["transcription", "chat", "speech"])
    );

    let id = app.start(Source::Chat).ok().unwrap();
    app.chat(id, "Сколько будет два плюс два?".into()).await;
    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], Value::Null);
    assert_eq!(state["messages"][3]["text"], "Четыре");

    app.plugins.shutdown().await;
    let id = app.start(Source::Chat).ok().unwrap();
    app.chat(id, "Ты здесь?".into()).await;
    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], "provider_unavailable");
}

use super::{
    Player, RecordingGain,
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
async fn voice_request_uses_the_transcription_gain_saved_in_the_panel() {
    const PROMPT: &str = "Ты домашний помощник.\nОтвечай дружелюбно.";
    let provider = Process::mock_rpc(|method, body| match method {
        "provider.transcribe" => {
            assert_eq!(body["language"], "ru");
            let wav = STANDARD
                .decode(body["wavBase64"].as_str().unwrap())
                .unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert!(
                wav[44..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|bytes| *bytes == 150_i16.to_le_bytes())
            );
            Ok(json!({ "text": "Сколько будет два плюс два?" }))
        }
        "provider.complete" => {
            let messages = body["messages"].as_array().unwrap();
            let text = messages.last().unwrap()["content"].as_str().unwrap();
            if text == "Fail the provider" {
                assert_eq!(messages[2]["content"], "Четыре");
                anyhow::bail!("Fixture provider failure");
            }
            assert_eq!(messages[0]["role"], "system");
            if text == "Что ты умеешь?" {
                assert_eq!(messages[0]["content"], PROMPT);
            } else {
                assert_eq!(text, "Сколько будет два плюс два?");
                assert_eq!(
                    messages[0]["content"],
                    format!("{PROMPT}\n\nUse plain text suitable for speaking aloud.")
                );
            }
            Ok(json!({"message": {"content": "Четыре"}}))
        }
        "provider.synthesize" => {
            assert_eq!(body["text"], "Четыре");
            Ok(
                json!({"pcmBase64": STANDARD.encode([1_u8, 2, 3, 4]), "sampleRate": 16_000, "channels": 1}),
            )
        }
        _ => panic!("Unexpected method {method}"),
    });

    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        access: Access::Open,
        settings: Settings {
            listening: false,
            ..Settings::default()
        },
        system_prompt: String::new(),
    };
    let sink = TestSink::default();
    let app = App::new(
        dir.path().into(),
        config,
        Player::recording_to(sink.clone()),
        DEFAULT_MICROPHONE_GAIN,
    );
    app.plugins.mock_provider("openrouter", provider);
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
        write_samples(&mut audio, 60, 12).await;
        write_samples(&mut audio, 0, 36).await;
        std::future::pending::<()>().await;
    });
    let capture = tokio::spawn(run_microphone(app.clone(), address));
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
        .put(format!("{panel}/api/settings/system-prompt"))
        .json(&json!({"system_prompt": PROMPT}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::NO_CONTENT);

    let mut settings = serde_json::to_value(app.snapshot()).unwrap()["settings"].clone();
    settings["wake_gain"] = json!(0.1);
    settings["transcription_gain"] = json!(2.5);
    let saved = client
        .put(format!("{panel}/api/settings"))
        .json(&json!({"settings": settings}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(saved["settings"]["wake_gain"], 0.1);
    assert_eq!(saved["settings"]["transcription_gain"], 2.5);
    let stored = Config::load(dir.path()).unwrap();
    assert_eq!(stored.settings.wake_gain, Some(0.1));
    assert_eq!(stored.settings.transcription_gain, Some(2.5));

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
        .json(&json!({"message": "Fail the provider"}))
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
    assert_eq!(state["last"]["error"], "provider_failed");
    assert_eq!(sink.lock().unwrap().len(), 1);

    let status = client
        .post(format!("{panel}/api/chat"))
        .json(&json!({"message": "Что ты умеешь?"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::ACCEPTED);
    wait_for(|| app.active().is_none()).await;
    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], Value::Null);

    for task in [capture, mic_task, panel_task] {
        task.abort();
    }
}

#[tokio::test]
async fn wake_word_without_a_transcribed_question_plays_a_local_retry_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let sink = TestSink::default();
    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
        Player::recording_to(sink.clone()),
        DEFAULT_MICROPHONE_GAIN,
    );
    app.plugins.mock_provider(
        "openrouter",
        Process::mock_rpc(|method, _| {
            assert_eq!(method, "provider.transcribe");
            Ok(json!({"text": "Гудвин."}))
        }),
    );
    app.set_microphone(true);
    app.set_model(true);
    let id = app.start(Source::Wake).ok().expect("start wake request");
    app.stage(id, Stage::Transcribing);
    app.voice(id, vec![128; 320], true, RecordingGain::Automatic(4.0))
        .await;

    let state = serde_json::to_value(app.snapshot()).unwrap();
    assert_eq!(state["last"]["error"], "no_speech");
    assert_eq!(state["messages"], json!([]));
    let played = sink.lock().unwrap();
    assert_eq!(played.len(), 1);
    assert_eq!(played[0], include_bytes!("../../assets/no-speech-ru.wav"));
}

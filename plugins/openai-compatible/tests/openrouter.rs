use axum::routing::get;
use axum::{
    Json, Router,
    http::{HeaderMap, header},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_openai_compatible::{Config, Protocol, Upstream};
use goodwin_plugin::provider::{
    Capability, ChatRequest, ModelSelection, SpeechRequest, ToolChoice, TranscriptionRequest,
};
use serde_json::{Value, json};

#[path = "../../../src/test_support.rs"]
mod test_support;

#[tokio::test]
async fn openrouter_uses_json_transcription_and_requires_tool_parameters() {
    let (url, server) = test_support::serve(
        Router::new()
            .route("/api/v1/audio/speech/models", get(|| async {
                Json(json!({"data":[
                    {"id":"fixture/selected-speech","name":"Selectable","output_modalities":[{"type":"speech","supported_parameters":{
                        "response_format":{"values":["pcm"]},"voice":{"values":["Fixture","Russian"]}
                    }}]},
                    {"id":"fixture/mp3-only","output_modalities":[{"type":"speech","supported_parameters":{"response_format":{"values":["mp3"]}}}]}
                ]}))
            }))
            .route("/api/v1/audio/transcriptions", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer fixture-key");
                assert_eq!(body["language"], "ru");
                assert_eq!(body["model"], "fixture/selected-recognition");
                assert_eq!(body["temperature"], 0.2);
                assert_eq!(body["input_audio"], json!({"data": STANDARD.encode(b"RIFF test"), "format": "wav"}));
                Json(json!({"text": "Привет"}))
            }))
            .route("/api/v1/chat/completions", post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "fixture/selected-chat");
                assert_eq!(body["temperature"], 0.7);
                assert_eq!(body["provider"], json!({"require_parameters": true}));
                assert_eq!(body["tool_choice"], "auto");
                assert_eq!(body["tools"][0]["function"]["name"], "fixture");
                Json(json!({"choices": [{"message": {"role": "assistant", "content": "Готово", "reasoning_details": ["fixture"]}}]}))
            }))
            .route("/api/v1/audio/speech", post(|Json(body): Json<Value>| async move {
                assert_eq!(body, json!({
                    "model": "fixture/selected-speech", "voice": "Russian", "speed": 1.1, "input": "Готово", "response_format": "pcm"
                }));
                ([(header::CONTENT_TYPE, "audio/pcm; rate=16000; channels=2")], [1_u8, 2, 3, 4])
            })),
    ).await;
    tokio::task::spawn_blocking(move || {
        let client = Upstream::new(Protocol::OpenRouter).unwrap();
        let config = Config {
            base_url: format!("{url}/api/v1"),
            key: "fixture-key".into(),
            transcription_model: "fixture/recognition".into(),
            chat_model: "fixture/chat".into(),
            speech_model: "fixture/speech".into(),
            voice: "Fixture".into(),
        };
        let catalog = client.models(&config, Capability::Speech);
        assert!(catalog.available);
        assert_eq!(catalog.default.as_deref(), Some("fixture/speech"));
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.models[1].voices, ["Fixture", "Russian"]);
        let transcript = client
            .transcribe(
                &config,
                TranscriptionRequest {
                    language: "ru".into(),
                    wav_base64: STANDARD.encode(b"RIFF test"),
                    selection: selection(
                        "fixture/selected-recognition",
                        json!({"temperature":0.2}),
                    ),
                },
            )
            .unwrap();
        assert_eq!(transcript.text, "Привет");
        let completion = client
            .complete(
                &config,
                ChatRequest {
                    messages: vec![json!({"role":"user","content":transcript.text})],
                    tools: vec![json!({"type":"function","function":{"name":"fixture"}})],
                    tool_choice: Some(ToolChoice::Auto),
                    max_completion_tokens: 256,
                    selection: selection("fixture/selected-chat", json!({"temperature":0.7})),
                },
            )
            .unwrap();
        assert_eq!(completion.message["reasoning_details"], json!(["fixture"]));
        let audio = client
            .synthesize(
                &config,
                SpeechRequest {
                    text: completion.message["content"].as_str().unwrap().to_owned(),
                    selection: selection(
                        "fixture/selected-speech",
                        json!({"voice":"Russian","speed":1.1}),
                    ),
                },
            )
            .unwrap();
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.channels, 2);
        assert_eq!(STANDARD.decode(audio.pcm_base64).unwrap(), [1, 2, 3, 4]);
    })
    .await
    .unwrap();
    server.abort();
}

fn selection(model: &str, parameters: Value) -> ModelSelection {
    ModelSelection {
        model: Some(model.into()),
        parameters: parameters.as_object().unwrap().clone(),
    }
}

use std::fs;

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Request as HttpWireRequest},
    http::{HeaderMap, header},
    response::Response,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use goodwin_plugin::{
    provider::{SpeechRequest, TranscriptionRequest},
    wake::{Frame, Model},
};
use tokio::sync::Notify;

use super::*;
use crate::plugins::connections;

macro_rules! wasm {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/target/wasm32-unknown-unknown/release/goodwin-",
            $name,
            ".wasm"
        ))
    };
}

fn install(root: &Path, mut manifest: Value, module: &[u8]) -> Manifest {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin/plugin.wasm"), module).unwrap();
    fs::create_dir_all(root.join("ui")).unwrap();
    fs::write(root.join("ui/index.mjs"), "export default null;").unwrap();
    fs::write(root.join("ui/style.css"), "").unwrap();
    manifest["target"] = json!("wasm32");
    manifest["executable"] = json!("bin/plugin.wasm");
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    Manifest::read(root).unwrap()
}

async fn plugin(root: &Path, data: &Path, manifest: &Manifest) -> (Process, Registration) {
    let (sound, _) = mpsc::channel(1);
    let (changes, _) = watch::channel(());
    Process::start(root, data, manifest, sound, changes)
        .await
        .unwrap()
}

fn save_connection_fixture(data: &Path, endpoint: &str, key: &str) {
    let mut store = connections::Store::default();
    store
        .update(
            serde_json::from_value(json!({
                "name": "Fixture", "protocol": "openai", "baseUrl": endpoint, "key": key,
            }))
            .unwrap(),
            || Ok("fixture".into()),
        )
        .ok()
        .unwrap();
    store.save(data).unwrap();
}

#[tokio::test]
async fn wasm_providers_use_host_http_for_transcription_models_and_live_streams() {
    for router_protocol in [false, true] {
        let first_chat = Arc::new(Notify::new());
        let first_speech = Arc::new(Notify::new());
        let chat = first_chat.clone();
        let speech = first_speech.clone();
        let router = Router::new()
            .route(
                "/v1/models",
                get(move |request: HttpWireRequest| async move {
                    assert_eq!(request.headers()[header::AUTHORIZATION], "Bearer fixture");
                    if router_protocol {
                        assert!(request.uri().query().unwrap().starts_with("endpoints="));
                    }
                    axum::Json(json!({"data":[{"id":"fixture"}]}))
                }),
            )
            .route(
                "/v1/audio/transcriptions",
                post(move |headers: HeaderMap, body: Bytes| async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer fixture");
                    let content_type = headers[header::CONTENT_TYPE].to_str().unwrap();
                    if router_protocol {
                        assert_eq!(content_type, "application/json");
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        let encoded = body["input_audio"]["data"].as_str().unwrap();
                        assert!(encoded.starts_with("UklGRg"));
                        assert_eq!(encoded.len(), MAX_RECORDING_BYTES.div_ceil(3) * 4);
                    } else {
                        assert!(content_type.starts_with("multipart/form-data;"));
                        assert!(body.windows(4).any(|bytes| bytes == b"RIFF"));
                        assert!(body.len() > MAX_RECORDING_BYTES);
                    }
                    axum::Json(json!({"text":"hello"}))
                }),
            )
            .route(
                "/v1/chat/completions",
                post(
                    move |headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
                        let chat = chat.clone();
                        async move {
                            assert_eq!(headers[header::AUTHORIZATION], "Bearer fixture");
                            assert_eq!(headers[header::ACCEPT], "text/event-stream");
                            assert_eq!(body["stream"], true);
                            streamed(
                                "text/event-stream",
                                b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
                                b"data: [DONE]\n\n",
                                chat,
                            )
                        }
                    },
                ),
            )
            .route(
                "/v1/audio/speech",
                post(move |headers: HeaderMap| {
                    let speech = speech.clone();
                    async move {
                        if headers
                            .get(header::ACCEPT)
                            .is_none_or(|value| value != "audio/pcm")
                        {
                            let silence = stream::iter(
                                (0..MAX_SPEECH_BYTES).step_by(16 * 1024).map(|offset| {
                                    static BLOCK: [u8; 16 * 1024] = [0; 16 * 1024];
                                    Ok::<_, std::io::Error>(Bytes::from_static(
                                        &BLOCK[..(MAX_SPEECH_BYTES - offset).min(BLOCK.len())],
                                    ))
                                }),
                            );
                            return Response::builder()
                                .header(header::CONTENT_TYPE, "audio/pcm")
                                .body(Body::from_stream(silence))
                                .unwrap();
                        }
                        streamed(
                            "audio/pcm;rate=16000;channels=2",
                            &[1, 2, 3, 4, 5],
                            &[6, 7, 8],
                            speech,
                        )
                    }
                }),
            )
            .layer(DefaultBodyLimit::max(6 * 1024 * 1024));
        let (server, task) = crate::test_support::serve(router).await;
        let root = tempfile::tempdir().unwrap();
        let manifest = install(
            root.path(),
            serde_json::from_str(include_str!(
                "../../../../plugins/openai-compatible/manifest.json"
            ))
            .unwrap(),
            wasm!("openai-compatible"),
        );
        let endpoint = format!("{server}/v1");
        // Load the fixture directly so OpenRouter traffic also uses the loopback server.
        let store: connections::Store = serde_json::from_value(json!({
            "revision": 1,
            "entries": {
                "fixture": {
                    "name": "Fixture",
                    "protocol": if router_protocol { "openrouter" } else { "openai" },
                    "base_url": endpoint,
                    "key": "fixture",
                },
            },
        }))
        .unwrap();
        store.save(root.path()).unwrap();

        let (process, registration) = plugin(root.path(), root.path(), &manifest).await;
        let provider = process.for_connection("fixture", !router_protocol);
        assert!(registration.chat_streaming && registration.model_catalog);
        let timer_root = tempfile::tempdir().unwrap();
        let timer_manifest = install(
            timer_root.path(),
            serde_json::from_str(include_str!("../../../../plugins/timers/manifest.json")).unwrap(),
            wasm!("timers"),
        );
        let (timers, _) = plugin(timer_root.path(), timer_root.path(), &timer_manifest).await;
        let mut wav = vec![0; MAX_RECORDING_BYTES];
        wav[..4].copy_from_slice(b"RIFF");
        let selection = serde_json::from_value(json!({"model":"fixture"})).unwrap();
        let transcript: goodwin_plugin::provider::Transcription = provider
            .provider(
                Capability::Transcription,
                TranscriptionRequest {
                    wav_base64: STANDARD.encode(wav),
                    language: "ru".into(),
                    selection,
                },
            )
            .await
            .unwrap();
        assert_eq!(transcript.text, "hello");
        let models = provider.models(Capability::Chat).await.unwrap();
        assert!(models.models.iter().any(|model| model.id == "fixture"));
        let mut chat = provider
            .stream_chat(
                &serde_json::from_value(json!({
                    "messages":[], "max_completion_tokens":16, "selection":{"model":"fixture"},
                }))
                .unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(8), chat.next())
                .await
                .unwrap()
                .unwrap()
                .as_deref(),
            Some("hello")
        );
        timeout(
            Duration::from_secs(2),
            timers.call("http", json!({"method":"GET","path":"/timers","body":null})),
        )
        .await
        .unwrap()
        .unwrap();
        first_chat.notify_one();
        assert!(chat.next().await.unwrap().is_none());
        if !router_protocol {
            let mut speech = provider
                .stream_speech(SpeechRequest {
                    text: "hello".into(),
                    selection: serde_json::from_value(json!({"model":"fixture"})).unwrap(),
                })
                .unwrap()
                .unwrap();
            let first = timeout(Duration::from_secs(8), speech.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!((first.sample_rate, first.channels), (16_000, 2));
            assert_eq!(STANDARD.decode(first.pcm_base64).unwrap(), [1, 2, 3, 4]);
            first_speech.notify_one();
            assert_eq!(
                STANDARD
                    .decode(speech.next().await.unwrap().unwrap().pcm_base64)
                    .unwrap(),
                [5, 6, 7, 8]
            );
            assert!(speech.next().await.unwrap().is_none());
            let buffered: SpeechAudio = provider
                .provider(
                    Capability::Speech,
                    SpeechRequest {
                        text: "hello".into(),
                        selection: serde_json::from_value(json!({"model":"fixture"})).unwrap(),
                    },
                )
                .await
                .expect("WASM provider returns a maximum-length buffered PCM reply");
            assert_eq!(buffered.pcm_base64.len(), MAX_SPEECH_BYTES.div_ceil(3) * 4);
        }
        provider.stop().await;
        timers.stop().await;
        task.abort();
    }
}

fn streamed(
    content_type: &'static str,
    first: &'static [u8],
    last: &'static [u8],
    gate: Arc<Notify>,
) -> Response {
    let chunks = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from_static(first)) })
        .chain(stream::once(async move {
            gate.notified().await;
            Ok(Bytes::from_static(last))
        }));
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from_stream(chunks))
        .unwrap()
}

#[tokio::test]
async fn stopping_wasm_plugin_cancels_http_header_and_body_waits() {
    for stall_headers in [true, false] {
        let entered = Arc::new(Notify::new());
        let upstream = entered.clone();
        let server = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let upstream = upstream.clone();
                async move {
                    if stall_headers {
                        upstream.notify_one();
                        return std::future::pending::<Response>().await;
                    }
                    streamed(
                        "text/event-stream",
                        b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
                        b"data: [DONE]\n\n",
                        upstream,
                    )
                }
            }),
        );
        let (address, server) = crate::test_support::serve(server).await;
        let root = tempfile::tempdir().unwrap();
        let manifest = install(
            root.path(),
            serde_json::from_str(include_str!(
                "../../../../plugins/openai-compatible/manifest.json"
            ))
            .unwrap(),
            wasm!("openai-compatible"),
        );
        save_connection_fixture(root.path(), &format!("{address}/v1"), "");
        let (process, _) = plugin(root.path(), root.path(), &manifest).await;
        let provider = process.for_connection("fixture", true);
        let mut chat = provider
            .stream_chat(
                &serde_json::from_value(json!({
                    "messages":[], "max_completion_tokens":16, "selection":{"model":"fixture"},
                }))
                .unwrap(),
            )
            .unwrap()
            .unwrap();
        if stall_headers {
            timeout(Duration::from_secs(8), entered.notified())
                .await
                .unwrap();
        } else {
            assert_eq!(
                timeout(Duration::from_secs(8), chat.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .as_deref(),
                Some("hello")
            );
        }
        timeout(Duration::from_secs(2), process.stop())
            .await
            .expect("Stopping the plugin cancels a stalled upstream request");
        assert!(chat.next().await.is_err());
        server.abort();
    }
}

#[tokio::test]
async fn named_connections_route_independently_and_persist_after_restart() {
    let router = Router::new()
        .route(
            "/first/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer first-key");
                axum::Json(json!({"data":[{"id":"first-model"}]}))
            }),
        )
        .route(
            "/second/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer second-key");
                axum::Json(json!({"data":[{"id":"second-model"}]}))
            }),
        );
    let (server, task) = crate::test_support::serve(router).await;
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("plugins/installed");
    let slot = installed.join("openai-compatible");
    fs::create_dir_all(slot.join("data")).unwrap();
    install(
        &slot.join("package"),
        serde_json::from_str(include_str!(
            "../../../../plugins/openai-compatible/manifest.json"
        ))
        .unwrap(),
        wasm!("openai-compatible"),
    );
    let plugins = crate::plugins::Plugins::new(root.path(), crate::station::Player::new());
    let router_provider = crate::assistant::DEFAULT_PROVIDER;
    plugins.restore_selected(&[router_provider]).await;
    assert!(
        plugins
            .provider(router_provider, Capability::Chat)
            .is_some()
    );
    assert!(
        plugins
            .provider("openai-compatible", Capability::Chat)
            .is_none()
    );

    let connections =
        serde_json::to_value(plugins.connections("openai-compatible").await.ok().unwrap()).unwrap();
    let [router] = connections.as_array().unwrap().as_slice() else {
        panic!("Expected the initial OpenRouter connection");
    };
    assert_eq!(router["providerId"], router_provider);
    assert_eq!(router["protocol"], "openrouter");
    assert_eq!(router["hasKey"], false);
    assert_eq!(router["speechStreaming"], false);
    assert!(router.get("key").is_none());

    let (first_id, _) = plugins.save_connection("openai-compatible", serde_json::from_value(json!({
        "name":"First server", "protocol":"openai", "baseUrl":format!("{server}/first"), "key":"first-key",
    })).unwrap()).await.ok().unwrap();
    let first_provider = format!("openai-compatible:{first_id}");
    let (id, views) = plugins.save_connection("openai-compatible", serde_json::from_value(json!({
        "name":"Second server", "protocol":"openai", "baseUrl":format!("{server}/second"), "key":"second-key",
    })).unwrap()).await.ok().unwrap();
    let provider = format!("openai-compatible:{id}");
    assert!(
        views
            .iter()
            .any(|connection| connection.provider_id == provider)
    );
    let first = plugins
        .models(&first_provider, Capability::Chat)
        .await
        .ok()
        .unwrap();
    let second = plugins
        .models(&provider, Capability::Chat)
        .await
        .ok()
        .unwrap();
    assert_eq!(first.models[0].id, "first-model");
    assert_eq!(second.models[0].id, "second-model");
    let (_, views) = plugins.save_connection("openai-compatible", serde_json::from_value(json!({
        "id": id, "name":"Renamed server", "protocol":"openai", "baseUrl":format!("{server}/second"),
    })).unwrap()).await.ok().unwrap();
    assert!(
        serde_json::to_string(&views)
            .unwrap()
            .contains("Renamed server")
    );
    let (stale, _) = plugins.provider(router_provider, Capability::Chat).unwrap();
    plugins
        .remove_connection("openai-compatible", "openrouter".into())
        .await
        .ok()
        .unwrap();
    assert!(
        plugins
            .provider(router_provider, Capability::Chat)
            .is_none()
    );
    let error = stale
        .models(Capability::Chat)
        .await
        .err()
        .expect("The removed connection rejects the stale request");
    assert!(error.to_string().contains("Plugin rejected the request"));
    assert!(stale.running());
    assert_eq!(
        plugins
            .models(&provider, Capability::Chat)
            .await
            .ok()
            .unwrap()
            .models[0]
            .id,
        "second-model"
    );
    plugins.shutdown().await;
    plugins.restore().await;
    assert!(
        plugins
            .provider(router_provider, Capability::Chat)
            .is_none()
    );
    assert_eq!(
        plugins
            .models(&provider, Capability::Chat)
            .await
            .ok()
            .unwrap()
            .models[0]
            .id,
        "second-model"
    );
    plugins.remove("openai-compatible").await.ok().unwrap();
    assert!(plugins.provider(&provider, Capability::Chat).is_none());
    plugins.shutdown().await;
    task.abort();
}

#[tokio::test]
async fn wasm_dataset_captures_audio_and_reopens_saved_clips() {
    let root = tempfile::tempdir().unwrap();
    let manifest = install(
        root.path(),
        serde_json::from_str(include_str!(
            "../../../../plugins/wake-dataset/manifest.json"
        ))
        .unwrap(),
        wasm!("wake-dataset"),
    );
    let data = tempfile::tempdir().unwrap();
    let (dataset, registration) = plugin(root.path(), data.path(), &manifest).await;
    assert!(registration.wake_events);
    assert_eq!(dataset.wake_subscription(), None);
    dataset
        .call(
            "http",
            json!({"method":"PUT", "path":"/capture", "body":{"enabled":true}}),
        )
        .await
        .unwrap();
    let subscription = dataset.wake_subscription().unwrap();
    dataset
        .observe_wake(
            subscription,
            &Event::Frame(Frame {
                stream: 10,
                index: 0,
                model: Model {
                    id: "fixture".into(),
                    wake_word: "Goodwin".into(),
                    kind: "micro".into(),
                    sha256: "abc".into(),
                },
                ended_at_ms: 1000,
                sample_rate: 16_000,
                gain: 2.0,
                threshold: 0.4,
                score: 0.5,
                pcm: STANDARD.encode((-20_000_i16).to_le_bytes().repeat(1280)),
            }),
        )
        .unwrap();
    dataset
        .observe_wake(subscription, &Event::End { stream: 10 })
        .unwrap();
    let list = dataset
        .call(
            "http",
            json!({"method":"GET", "path":"/clips", "body":null}),
        )
        .await
        .unwrap();
    let id = list["body"]["clips"][0]["id"].as_u64().unwrap();
    assert_eq!(list["body"]["clips"][0]["info"]["gainApplied"], true);
    dataset
        .call(
            "http",
            json!({"method":"PUT", "path":format!("/clips/{id}"), "body":{"label":"positive"}}),
        )
        .await
        .unwrap();
    dataset.stop().await;
    let (dataset, _) = plugin(root.path(), data.path(), &manifest).await;
    assert_eq!(dataset.wake_subscription(), None);
    assert!(
        data.path()
            .join(format!("clips/positive/{id}.clip"))
            .is_file()
    );
    let audio = dataset
        .call(
            "http",
            json!({"method":"GET", "path":format!("/clips/{id}/audio"), "body":null}),
        )
        .await
        .unwrap();
    let wav = STANDARD
        .decode(audio["body"]["wav"].as_str().unwrap())
        .unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 44 + 2560);
    assert_eq!(&wav[44..], &i16::MIN.to_le_bytes().repeat(1280));
    dataset.stop().await;
}

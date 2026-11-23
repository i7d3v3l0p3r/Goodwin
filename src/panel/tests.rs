use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use super::router;
use crate::{
    app::App,
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    plugins::Process,
    station::Player,
    test_support::serve,
};

#[tokio::test]
async fn panel_lists_models_for_the_requested_provider_and_stage() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
    );
    app.plugins.mock_provider(
        "fixture",
        Process::mock_rpc(|method, body| {
            assert_eq!(method, "provider.models");
            assert_eq!(body["capability"], "speech");
            Ok(json!({
                "models":[{"id":"fixture/tts","name":"Fixture","voices":["Russian"]}],
                "default":null,
                "available":true,
            }))
        }),
    );
    let (panel, task) = serve(router(app)).await;
    let result: Value = reqwest::get(format!("{panel}/api/plugin-models/fixture/speech"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["models"][0]["id"], "fixture/tts");
    assert_eq!(result["models"][0]["voices"], json!(["Russian"]));
    task.abort();
}

#[tokio::test]
async fn panel_serves_direct_navigation_to_settings() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        access: Access::Open,
        settings: Settings::default(),
        system_prompt: String::new(),
    };
    let app = App::new(
        dir.path().into(),
        config,
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
    );
    let (panel, task) = serve(router(app)).await;
    let client = reqwest::Client::new();
    for path in [
        "/",
        "/settings",
        "/settings/?from=bookmark",
        "/plugins",
        "/plugins/timers",
        "/plugins/fixture/settings",
    ] {
        let response = client.get(format!("{panel}{path}")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert!(response.text().await.unwrap().contains("/panel.js"));
    }
    let response = client
        .get(format!("{panel}/plugin-runtime-v1.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert!(response.text().await.unwrap().contains("createSignal"));
    let response = client
        .get(format!("{panel}/api/settings-typo"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = client
        .post(format!("{panel}/api/setup"))
        .json(&serde_json::json!({"password": "test password", "confirmation": "test password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = client
        .post(format!("{panel}/api/plugin-packages"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"],
        "plugin_password_required"
    );
    task.abort();
}

#[tokio::test]
async fn panel_validates_persists_and_resets_the_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
    );
    let (panel, task) = serve(router(app)).await;
    let client = reqwest::Client::new();
    let url = format!("{panel}/api/settings/system-prompt");

    for (input, status, expected) in [
        (
            "  Говори на ты.\r\nНа вопросы отвечай спокойно.  ".into(),
            StatusCode::NO_CONTENT,
            "Говори на ты.\nНа вопросы отвечай спокойно.",
        ),
        (
            "я".repeat(4_001),
            StatusCode::BAD_REQUEST,
            "Говори на ты.\nНа вопросы отвечай спокойно.",
        ),
        (" \t\n".into(), StatusCode::NO_CONTENT, ""),
    ] {
        let response = client
            .put(&url)
            .json(&json!({"system_prompt": input}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::BAD_REQUEST {
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"],
                "invalid_system_prompt"
            );
        }
        let state = client
            .get(format!("{panel}/api/state"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(state["system_prompt"], expected);
        assert_eq!(Config::load(dir.path()).unwrap().system_prompt, expected);
    }
    task.abort();
}

#[tokio::test]
async fn panel_configures_a_plugin_without_exposing_the_saved_secret() {
    let dir = tempfile::tempdir().unwrap();
    let slot = dir.path().join("plugins/installed/fixture");
    let package = slot.join("package");
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::create_dir(slot.join("data")).unwrap();
    std::fs::write(package.join("bin/fixture"), b"unavailable plugin").unwrap();
    std::fs::write(
        package.join("manifest.json"),
        json!({
            "id": "fixture",
            "version": "1.0.0",
            "target": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            "apiVersion": 1,
            "executable": "bin/fixture",
            "title": {"en":"Fixture","ru":"Плагин"},
            "settings": [
                {"id":"endpoint","type":"url","required":true,"label":{"en":"URL","ru":"URL"}},
                {"id":"token","type":"secret","label":{"en":"Token","ru":"Токен"}}
            ]
        })
        .to_string(),
    )
    .unwrap();

    let app = App::new(
        dir.path().into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
    );
    app.plugins.restore().await;
    let (panel, task) = serve(router(app)).await;
    let client = reqwest::Client::new();
    let url = format!("{panel}/api/plugin-settings/fixture");
    let initial = client
        .get(&url)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(initial["fields"].as_array().unwrap().len(), 2);
    assert_eq!(initial["values"], json!({}));
    assert_eq!(initial["configuredSecrets"], json!([]));

    let saved = client
        .patch(&url)
        .json(&json!({"endpoint":"http://localhost:8000/v1","token":"fixture-secret"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        saved["values"],
        json!({"endpoint":"http://localhost:8000/v1"})
    );
    assert_eq!(saved["configuredSecrets"], json!(["token"]));
    assert!(!saved.to_string().contains("fixture-secret"));
    assert_eq!(
        client
            .patch(&url)
            .json(&json!({"endpoint": null, "token": null}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let persisted = client
        .get(&url)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(persisted, saved);

    let removed = client
        .patch(&url)
        .json(&json!({"token":null}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(removed["configuredSecrets"], json!([]));
    assert_eq!(removed["values"], saved["values"]);
    task.abort();
}

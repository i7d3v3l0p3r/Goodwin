use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use super::router;
use crate::{
    app::App,
    config::{Access, Config, Settings},
    openrouter::OpenRouter,
    station::Player,
    test_support::serve,
};

#[tokio::test]
async fn panel_serves_direct_navigation_to_settings() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        access: Access::Open,
        settings: Settings::default(),
        key: String::new(),
        system_prompt: String::new(),
    };
    let app = App::new(
        dir.path().into(),
        config,
        OpenRouter::new().unwrap(),
        Player::new(),
    );
    let (panel, task) = serve(router(app)).await;
    let client = reqwest::Client::new();
    for path in [
        "/",
        "/settings",
        "/settings/?from=bookmark",
        "/plugins",
        "/plugins/timers",
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
            key: String::new(),
            system_prompt: String::new(),
        },
        OpenRouter::new().unwrap(),
        Player::new(),
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

use axum::http::{StatusCode, header};
use serde_json::json;

use crate::{
    app::App,
    config::{Access, Config, Settings, hash_password},
    openrouter::OpenRouter,
    panel::router,
    station::Player,
    test_support::serve,
};

#[tokio::test]
async fn password_mode_protects_settings_and_logout_invalidates_session() {
    let dir = tempfile::tempdir().unwrap();
    let password = "correct test password";
    let config = Config {
        access: Access::Password {
            hash: hash_password(password).unwrap(),
        },
        settings: Settings::default(),
        key: "initial-key".into(),
    };
    config.save(dir.path(), true).unwrap();
    let app = App::new(
        dir.path().into(),
        config,
        OpenRouter::new().unwrap(),
        Player::new(),
    );
    let (panel, task) = serve(router(app)).await;
    let client = reqwest::Client::new();
    let state_url = format!("{panel}/api/state");
    assert_eq!(
        client.get(&state_url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .post(format!("{panel}/api/login"))
        .json(&json!({"password": password}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let cookie = cookie.split(';').next().unwrap();
    let settings_url = format!("{panel}/api/settings");
    let settings = Settings {
        listening: false,
        ..Settings::default()
    };
    let response = client
        .put(&settings_url)
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://other.example")
        .json(&json!({"settings": settings}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = client
        .put(&settings_url)
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, &panel)
        .json(&json!({"settings": settings, "key": "new-key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.text().await.unwrap().contains("new-key"));
    let saved = Config::load(dir.path()).unwrap();
    assert_eq!(saved.key, "new-key");
    assert!(!saved.settings.listening);
    assert_eq!(
        client
            .post(format!("{panel}/api/logout"))
            .header(header::COOKIE, cookie)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .get(&state_url)
            .header(header::COOKIE, cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

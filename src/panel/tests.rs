use axum::http::{StatusCode, header};
use serde_json::Value;

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
        "/plugins/probe",
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

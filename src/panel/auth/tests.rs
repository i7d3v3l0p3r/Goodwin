use std::{fs, path::Path, time::SystemTime};

use axum::http::{StatusCode, header};
use serde_json::json;
use tokio::task::JoinHandle;

use crate::{
    app::App,
    config::{Access, Config, Settings, hash_password},
    openrouter::OpenRouter,
    panel::router,
    station::Player,
    test_support::serve,
};

use super::{Auth, SESSION_FILE};

const PASSWORD: &str = "correct test password";

fn setup(dir: &Path) {
    let config = Config {
        access: Access::Password {
            hash: hash_password(PASSWORD).unwrap(),
        },
        settings: Settings::default(),
        key: "initial-key".into(),
    };
    config.save(dir, true).unwrap();
}

async fn start_panel(dir: &Path) -> (String, JoinHandle<()>) {
    serve(router(App::new(
        dir.into(),
        Config::load(dir).unwrap(),
        OpenRouter::new().unwrap(),
        Player::new(),
    )))
    .await
}

async fn login(client: &reqwest::Client, panel: &str) -> String {
    let response = client
        .post(format!("{panel}/api/login"))
        .json(&json!({"password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Max-Age=43200"));
    cookie.split(';').next().unwrap().to_owned()
}

async fn state_status(client: &reqwest::Client, panel: &str, cookie: &str) -> StatusCode {
    client
        .get(format!("{panel}/api/state"))
        .header(header::COOKIE, cookie)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn password_mode_protects_settings_and_session_survives_restart_until_logout() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    assert_eq!(
        state_status(&client, &panel, "").await,
        StatusCode::UNAUTHORIZED
    );
    let cookie = login(&client, &panel).await;
    let token = cookie.split_once('=').unwrap().1;
    let stored = fs::read_to_string(dir.path().join(SESSION_FILE)).unwrap();
    assert!(!stored.contains(token));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(state_status(&client, &panel, &cookie).await, StatusCode::OK);
    let settings_url = format!("{panel}/api/settings");
    let settings = Settings {
        listening: false,
        ..Settings::default()
    };
    let response = client
        .put(&settings_url)
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, "http://other.example")
        .json(&json!({"settings": settings}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = client
        .put(&settings_url)
        .header(header::COOKIE, &cookie)
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
            .header(header::COOKIE, &cookie)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        state_status(&client, &panel, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(
        state_status(&client, &panel, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

#[tokio::test]
async fn expired_session_is_rejected_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &panel).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let mut auth = Auth::load(dir.path());
    let mut sessions = auth.sessions.clone();
    assert_eq!(sessions.len(), 1);
    for deadline in sessions.values_mut() {
        *deadline = SystemTime::UNIX_EPOCH;
    }
    auth.save(dir.path(), sessions).unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(
        state_status(&client, &panel, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

#[tokio::test]
async fn changing_the_password_revokes_saved_sessions() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &panel).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let mut config = Config::load(dir.path()).unwrap();
    config.access = Access::Password {
        hash: hash_password("replacement test password").unwrap(),
    };
    config.save(dir.path(), false).unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(
        state_status(&client, &panel, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use axum::http::{StatusCode, header};
use serde_json::json;
use tokio::task::JoinHandle;

use crate::{
    app::App,
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    panel::router,
    password,
    station::Player,
    test_support::serve,
};

use super::{Auth, SESSION_FILE, session_key};

const PASSWORD: &str = "correct test password";

fn setup(dir: &Path) {
    let config = Config {
        access: Access::Password {
            hash: password::hash(PASSWORD).unwrap(),
        },
        settings: Settings::default(),
        system_prompt: String::new(),
    };
    config.save(dir, true).unwrap();
}

async fn start_panel(dir: &Path) -> (String, JoinHandle<()>) {
    serve(router(App::new(
        dir.into(),
        Config::load(dir).unwrap(),
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
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
async fn first_visit_sets_one_password_and_signs_in() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        access: Access::PendingPassword,
        settings: Settings {
            listening: false,
            ..Settings::default()
        },
        system_prompt: String::new(),
    }
    .save(dir.path(), true)
    .unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    let access = || async {
        client
            .get(format!("{panel}/api/access"))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()
    };
    assert_eq!(
        access().await,
        json!({
            "protected": true, "authorized": false, "setup_required": true
        })
    );
    assert_eq!(
        state_status(&client, &panel, "").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .post(format!("{panel}/api/login"))
            .json(&json!({"password": PASSWORD}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    let submit = |password: &str, confirmation: &str| {
        client
            .post(format!("{panel}/api/setup"))
            .json(&json!({"password": password, "confirmation": confirmation}))
    };
    assert_eq!(
        submit(PASSWORD, PASSWORD)
            .header(header::ORIGIN, "http://other.example")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for (password, confirmation, code) in [
        ("short", "short", "invalid_password"),
        (PASSWORD, "another password", "password_mismatch"),
    ] {
        let response = submit(password, confirmation).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap(),
            json!({"error": code})
        );
    }

    let other_password = "another valid password";
    let (first, second) = tokio::join!(
        submit(PASSWORD, PASSWORD).send(),
        submit(other_password, other_password).send()
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    let (winner, rejected, password) = if first.status() == StatusCode::NO_CONTENT {
        (first, second, PASSWORD)
    } else {
        (second, first, other_password)
    };
    assert_eq!(winner.status(), StatusCode::NO_CONTENT);
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert!(!rejected.headers().contains_key(header::SET_COOKIE));
    let cookie = winner.headers()[header::SET_COOKIE].to_str().unwrap();
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        access().await,
        json!({
            "protected": true, "authorized": false, "setup_required": false
        })
    );
    assert_eq!(state_status(&client, &panel, cookie).await, StatusCode::OK);

    let saved = Config::load(dir.path()).unwrap();
    let Access::Password { hash } = saved.access else {
        panic!("expected a password hash")
    };
    assert!(password::verify(&hash, password).unwrap());
    assert!(!saved.settings.listening);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(state_status(&client, &panel, cookie).await, StatusCode::OK);
    assert_eq!(
        state_status(&client, &panel, "").await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

#[tokio::test]
async fn legacy_password_and_session_require_a_local_password_reset() {
    for hash in [
        "$argon2id$v=19$m=19456,t=2,p=1$AAECAwQFBgcICQoLDA0ODw$old",
        "$pbkdf2-sha512$i=220000$AAECAwQFBgcICQoLDA0ODw$IuRYD2bPNZecS5FyOX+tYXMm56zoVBeMs/h2WUwIk1qXPNZn8EDGKp8XX/sfBTgfckGcEqBPlFuGWKYizXGemA",
    ] {
        assert_password_reset_required(hash).await;
    }
}

async fn assert_password_reset_required(legacy_hash: &str) {
    const TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        access: Access::Password {
            hash: legacy_hash.into(),
        },
        settings: Settings::default(),
        system_prompt: String::new(),
    };
    config.save(dir.path(), true).unwrap();
    Auth::default()
        .save(
            dir.path(),
            [(
                session_key(legacy_hash, TOKEN),
                SystemTime::now() + Duration::from_secs(60),
            )]
            .into(),
        )
        .unwrap();

    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    let cookie = crate::panel_core::session_cookie(TOKEN, 60);
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        state_status(&client, &panel, cookie).await,
        StatusCode::UNAUTHORIZED
    );
    let rejected = client
        .post(format!("{panel}/api/login"))
        .json(&json!({"password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert_eq!(
        rejected.json::<serde_json::Value>().await.unwrap(),
        json!({"error": "password_reset_required"})
    );
    let setup = client
        .post(format!("{panel}/api/setup"))
        .json(&json!({"password": PASSWORD, "confirmation": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::CONFLICT);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    config.access = Access::Password {
        hash: password::hash(PASSWORD).unwrap(),
    };
    config.save(dir.path(), false).unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(
        state_status(&client, &panel, cookie).await,
        StatusCode::UNAUTHORIZED
    );
    let cookie = login(&client, &panel).await;
    assert_eq!(state_status(&client, &panel, &cookie).await, StatusCode::OK);
    task.abort();
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
    let response = client
        .put(format!("{settings_url}/system-prompt"))
        .json(&json!({"system_prompt": "Untrusted prompt"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
        .json(&json!({"settings": settings}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = Config::load(dir.path()).unwrap();
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
async fn diagnostic_logs_require_a_signed_in_session_and_disable_caching() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    fs::write(dir.path().join("goodwin.log"), "Goodwin journal fixture\n").unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    let client = reqwest::Client::new();
    let logs = format!("{panel}/api/diagnostics/logs");
    let unauthorized = client.get(&logs).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let cookie = login(&client, &panel).await;
    let response = client
        .get(&logs)
        .header(header::COOKIE, cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["supported"], true);
    assert!(body.get("kernel").is_some());
    assert_eq!(body["goodwin"], "Goodwin journal fixture");
    task.abort();
}

#[tokio::test]
async fn private_plugin_routes_require_a_signed_in_session() {
    let dir = tempfile::tempdir().unwrap();
    let start = |access| async {
        let app = App::new(
            dir.path().into(),
            Config {
                access,
                settings: Settings::default(),
                system_prompt: String::new(),
            },
            Player::new(),
            DEFAULT_MICROPHONE_GAIN,
        );
        app.plugins.mock_plugin(
            "fixture",
            crate::plugins::Process::mock_rpc(|_, _| {
                Ok(json!({"status": 200, "body": {"private": true}}))
            }),
            json!({"apiVersion": 1, "routes": [
                {"method": "GET", "path": "/private", "password_required": true}
            ]}),
        );
        serve(router(app)).await
    };
    let (panel, task) = start(Access::Open).await;
    let response = reqwest::get(format!("{panel}/api/plugins/fixture/private"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "plugin_password_required");
    task.abort();

    let (panel, task) = start(Access::Password {
        hash: password::hash(PASSWORD).unwrap(),
    })
    .await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &panel).await;
    let response = client
        .get(format!("{panel}/api/plugins/fixture/private"))
        .header(header::COOKIE, cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
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
        hash: password::hash("replacement test password").unwrap(),
    };
    config.save(dir.path(), false).unwrap();
    let (panel, task) = start_panel(dir.path()).await;
    assert_eq!(
        state_status(&client, &panel, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

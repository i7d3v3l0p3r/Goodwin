use std::{path::Path, time::Duration};

use axum::http::{StatusCode, header};
use serde_json::{Value, json};
use tokio::{sync::mpsc, time::timeout};

use crate::{
    app::App,
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    panel::router,
    plugins::Process,
    station::Player,
    test_support::serve,
};

struct EventReader {
    response: reqwest::Response,
    buffer: Vec<u8>,
}

impl EventReader {
    async fn connect(url: &str, cookie: Option<&str>) -> Self {
        let mut request = reqwest::Client::new().get(format!("{url}/api/events"));
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let response = request.send().await.unwrap().error_for_status().unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        Self {
            response,
            buffer: Vec::new(),
        }
    }

    async fn next(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                let frame: Vec<_> = self.buffer.drain(..end + 2).collect();
                let frame = String::from_utf8(frame).unwrap();
                let name = frame.lines().find_map(|line| line.strip_prefix("event: "));
                let data = frame.lines().find_map(|line| line.strip_prefix("data: "));
                if let (Some(name), Some(data)) = (name, data) {
                    return Some((name.into(), serde_json::from_str(data).unwrap()));
                }
            } else {
                self.buffer
                    .extend_from_slice(&self.response.chunk().await.unwrap()?);
            }
        }
    }

    async fn event(&mut self, name: &str) -> Value {
        let (actual, data) = timeout(Duration::from_secs(3), self.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual, name);
        data
    }
}

fn app(dir: &Path, config: Config) -> std::sync::Arc<App> {
    App::new(dir.into(), config, Player::new(), DEFAULT_MICROPHONE_GAIN)
}

#[tokio::test]
async fn connected_panels_get_state_changes_and_stay_quiet_in_idle() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
    );
    let (url, server) = serve(router(app.clone())).await;
    let mut first = EventReader::connect(&url, None).await;
    let mut second = EventReader::connect(&url, None).await;
    for browser in [&mut first, &mut second] {
        assert_eq!(browser.event("access").await["authorized"], true);
        assert_eq!(browser.event("state").await["microphone"], false);
    }
    app.microphone_disconnected();
    assert!(
        timeout(Duration::from_millis(850), first.next())
            .await
            .is_err()
    );

    assert!(app.microphone_connected(&app.microphone_mute()));
    for browser in [&mut first, &mut second] {
        assert_eq!(browser.event("state").await["microphone"], true);
    }
    app.configuration()
        .update_system_prompt("New instructions".to_owned())
        .unwrap();
    assert_eq!(
        first.event("state").await["system_prompt"],
        "New instructions"
    );
    app.stop_panel();
    assert!(
        timeout(Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn a_logged_out_session_stops_receiving_state_on_an_open_connection() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        Config {
            access: Access::PendingPassword,
            settings: Settings::default(),
            system_prompt: "private".to_owned(),
        },
    );
    let (url, server) = serve(router(app.clone())).await;
    let mut guest = EventReader::connect(&url, None).await;
    let access = guest.event("access").await;
    assert_eq!(access["setup_required"], true);
    assert_eq!(access["authorized"], false);

    let client = reqwest::Client::new();
    let setup = client
        .post(format!("{url}/api/setup"))
        .json(&json!({"password": "test password", "confirmation": "test password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::NO_CONTENT);
    let cookie = setup.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let access = guest.event("access").await;
    assert_eq!(access["setup_required"], false);
    assert_eq!(access["authorized"], false);

    let mut owner = EventReader::connect(&url, Some(cookie)).await;
    assert_eq!(owner.event("access").await["authorized"], true);
    assert_eq!(owner.event("state").await["system_prompt"], "private");
    let logout = client
        .post(format!("{url}/api/logout"))
        .header(header::COOKIE, cookie)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(owner.event("access").await["authorized"], false);
    app.configuration()
        .update_system_prompt("still private".to_owned())
        .unwrap();
    assert!(
        timeout(Duration::from_millis(150), owner.next())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(150), guest.next())
            .await
            .is_err()
    );
    server.abort();
}

#[tokio::test]
async fn reconnecting_during_chat_resumes_deltas_and_keeps_an_interrupted_reply() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.assistant.pipeline.chat.provider = "fixture".into();
    let app = app(
        dir.path(),
        Config {
            access: Access::Open,
            settings,
            system_prompt: String::new(),
        },
    );
    let (send, receive) = mpsc::channel(1);
    app.plugins
        .mock_provider("fixture", Process::mock_chat_stream(receive));
    let (url, server) = serve(router(app)).await;
    let mut first = EventReader::connect(&url, None).await;
    first.event("access").await;
    first.event("state").await;
    let response = reqwest::Client::new()
        .post(format!("{url}/api/chat"))
        .json(&json!({"message": "Привет"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    while first.event("state").await["messages"]
        .as_array()
        .unwrap()
        .is_empty()
    {}

    send.send(Ok("Здравствуйте".into())).await.unwrap();
    let first_delta = first.event("chat_delta").await;
    assert_eq!(first_delta["text"], "Здравствуйте");
    let mut second = EventReader::connect(&url, None).await;
    second.event("access").await;
    let draft = second.event("state").await["chat_draft"].clone();
    assert_eq!(draft["id"], first_delta["id"]);
    assert_eq!(draft["text"], "Здравствуйте");

    send.send(Ok(", мир".into())).await.unwrap();
    for reader in [&mut first, &mut second] {
        assert_eq!(
            reader.event("chat_delta").await,
            json!({"id": draft["id"], "text": ", мир"})
        );
    }
    send.send(Err(anyhow::anyhow!("upstream disconnected")))
        .await
        .unwrap();
    for reader in [&mut first, &mut second] {
        let state = reader.event("state").await;
        assert_eq!(
            state["messages"][1],
            json!({
                "id": draft["id"], "role": "assistant", "text": "Здравствуйте, мир", "error": "chat_interrupted",
            })
        );
        assert!(state["chat_draft"].is_null());
        assert!(state["active"].is_null());
    }
    server.abort();
}

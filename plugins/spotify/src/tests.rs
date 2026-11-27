use super::*;
use axum::{
    Json, Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[path = "../../../src/test_support.rs"]
mod test_support;

const TRACK: &str = "spotify:track:1234567890123456789012";

#[tokio::test]
async fn player_shows_the_receiver_name_before_pairing_without_sending_a_command() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = soloist::address(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let receiver = tokio::spawn(async move {
        let mut ws = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        ws.send(Message::text(
            r#"{"type":"auth_state","logged_in":false,"device_name":"Goodwin Mini 3"}"#,
        ))
        .await
        .unwrap();
        assert!(!matches!(ws.next().await, Some(Ok(Message::Text(_)))));
    });
    assert_eq!(
        player::snapshot(&url).await.unwrap(),
        json!({"connection":"pairing","device":"Goodwin Mini 3"})
    );
    receiver.await.unwrap();
}

#[tokio::test]
async fn player_reads_current_track_and_both_queue_directions_without_changing_playback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = soloist::address(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let receiver = tokio::spawn(async move {
        let mut ws = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        ws.send(Message::text(
            r#"{"type":"auth_state","logged_in":true,"device_name":"Studio"}"#,
        ))
        .await
        .unwrap();
        let request = ws.next().await.unwrap().unwrap().into_data();
        assert_eq!(
            serde_json::from_slice::<Value>(&request).unwrap(),
            json!({"type":"command","command":"get_queue","limit":20})
        );
        let queue = json!({"type":"queue_changed","previous":[{"item":{"uri":"previous-1"}},{"item":{"uri":"previous-2"}}],
            "upcoming":[{"item":{"uri":"next-1"}},{"item":{"uri":"next-1"}}]});
        ws.send(Message::text(queue.to_string())).await.unwrap();
        let playback = json!({"type":"playback_state","status":"paused","is_active":true,
            "item":{"uri":TRACK,"decorations":{"identity":{"name":"Track"},"creators":[{"entity":{"decorations":{"identity":{"name":"Artist"}}}}],
                "playback":{"duration_ms":90000},"parent":{"entity":{"decorations":{"identity":{"name":"Album"}}}}}},
            "position":{"position_ms":12000,"speed":0},"available_actions":{"play":{},"seek":{},"skip_next":{}}});
        ws.send(Message::text(playback.to_string())).await.unwrap();
    });
    let snapshot = player::snapshot(&url).await.unwrap();
    assert_eq!(snapshot["device"], "Studio");
    assert_eq!(
        snapshot["player"]["track"],
        json!({"uri":TRACK,"title":"Track","artists":["Artist"],"album":"Album","duration":90000})
    );
    assert_eq!(snapshot["player"]["position"], 12000);
    assert_eq!(snapshot["player"]["controls"], json!(["play", "skip_next"]));
    assert_eq!(snapshot["previous"][0]["uri"], "previous-1");
    assert_eq!(snapshot["previous"][1]["uri"], "previous-2");
    assert_eq!(snapshot["upcoming"][0]["uri"], "next-1");
    assert_eq!(snapshot["upcoming"][1]["uri"], "next-1");
    receiver.await.unwrap();
}

fn settings() -> Settings {
    Settings {
        client_id: "app".into(),
        client_secret: "secret".into(),
        market: "de".into(),
        ..Settings::default()
    }
}

fn catalog(base: &str) -> Catalog {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    Catalog::new(client, &format!("{base}/token"), &format!("{base}/search"))
}

#[tokio::test]
async fn spoken_title_and_artist_play_the_first_available_track_on_the_configured_receiver() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let issued = tokens.clone();
    let api = Router::new()
        .route(
            "/token",
            post(move |headers: HeaderMap, body: String| {
                issued.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert_eq!(headers["authorization"], "Basic YXBwOnNlY3JldA==");
                    assert_eq!(body, "grant_type=client_credentials");
                    Json(json!({"access_token":"access","expires_in":3600}))
                }
            }),
        )
        .route(
            "/search",
            get(
                |headers: HeaderMap, Query(args): Query<HashMap<String, String>>| async move {
                    assert_eq!(headers["authorization"], "Bearer access");
                    assert_eq!(args["q"], "Кино Группа крови");
                    assert_eq!(args["type"], "track");
                    assert_eq!(args["market"], "DE");
                    Json(json!({"tracks":{"items":[null,
                        {"uri":"spotify:track:aaaaaaaaaaaaaaaaaaaaaa","is_playable":false},
                        {"uri":TRACK}
                    ]}}))
                },
            ),
        );
    let (base, api) = test_support::serve(api).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let settings = Settings {
        soloist_url: format!("ws://{}", listener.local_addr().unwrap()),
        ..settings()
    };
    let receiver = tokio::spawn(async move {
        for _ in 0..2 {
            let mut ws = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            ws.send(Message::text(r#"{"type":"auth_state","logged_in":true}"#))
                .await
                .unwrap();
            let command = ws.next().await.unwrap().unwrap();
            let data: Value = serde_json::from_slice(&command.into_data()).unwrap();
            assert_eq!(
                data,
                json!({"type":"command", "command":"play", "uri":TRACK})
            );
            ws.send(Message::text(
                r#"{"type":"playback_state","status":"paused"}"#,
            ))
            .await
            .unwrap();
            ws.send(Message::text(
                r#"{"type":"command_result","command":"play"}"#,
            ))
            .await
            .unwrap();
        }
    });
    let catalog = Mutex::new(catalog(&base));
    for _ in 0..2 {
        let action = Action::parse(&ToolCall {
            name: "play".into(),
            arguments: json!({"query":"Кино Группа крови"}),
            voice_turn: 0,
        })
        .unwrap();
        execute(&catalog, &settings, action).await.unwrap();
    }
    receiver.await.unwrap();
    assert_eq!(tokens.load(Ordering::SeqCst), 1);
    api.abort();
}

#[tokio::test]
async fn expired_token_is_replaced_and_empty_results_never_reach_the_receiver() {
    let count = Arc::new(AtomicUsize::new(0));
    let issued = count.clone();
    let api = Router::new()
        .route("/token", post(move || {
            let n = issued.fetch_add(1, Ordering::SeqCst);
            async move { Json(json!({"access_token": format!("token{n}"), "expires_in": 3600})) }
        }))
        .route("/search", get(|headers: HeaderMap| async move {
            if headers["authorization"] == "Bearer token0" {
                (StatusCode::UNAUTHORIZED, Json(Value::Null))
            } else {
                (StatusCode::OK, Json(json!({"albums":{"items":[]}})))
            }
        }));
    let (base, api) = test_support::serve(api).await;
    assert_eq!(
        execute(
            &Mutex::new(catalog(&base)),
            &settings(),
            Action::Search {
                query: "example".into(),
                kind: "album".into()
            }
        )
        .await,
        Err("no_playable_match")
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    api.abort();
}

#[tokio::test]
async fn logged_out_receiver_requests_pairing_and_receives_no_command() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = soloist::address(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let receiver = tokio::spawn(async move {
        let mut ws = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        ws.send(Message::text(r#"{"type":"auth_state","logged_in":false}"#))
            .await
            .unwrap();
        assert!(!matches!(ws.next().await, Some(Ok(Message::Text(_)))));
    });
    assert_eq!(
        soloist::command(&url, "play", None).await,
        Err("select_soloist_in_spotify")
    );
    receiver.await.unwrap();
}

#[tokio::test]
async fn new_requests_cancel_an_older_pending_search() {
    let mut requests = Requests::default();
    let (finish, received) = tokio::sync::oneshot::channel();
    assert_eq!(
        requests.submit(async { received.await.unwrap() }).await,
        json!({"status":"pending"})
    );
    assert_eq!(
        requests
            .submit(async { Err("soloist_rejected_command") })
            .await,
        json!({"error":"soloist_rejected_command"})
    );
    assert!(finish.send(Ok(())).is_err());
    assert_eq!(
        requests.status().await,
        json!({"error":"soloist_rejected_command"})
    );
}

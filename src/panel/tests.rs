use std::io::Read;

use axum::http::{StatusCode, header};
use flate2::read::GzDecoder;
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
async fn wake_model_ticket_reports_pending_and_then_the_active_selection() {
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
    let (ticket, expected) = app.wake_models.stage_fixture("fixture");
    let (panel, task) = serve(router(app.clone())).await;
    let client = reqwest::Client::new();
    let result = || {
        client
            .post(format!("{panel}/api/wake-models/result"))
            .json(&json!({"operation": ticket}))
    };
    let pending = result().send().await.unwrap();
    assert_eq!(pending.status(), StatusCode::ACCEPTED);
    assert_eq!(
        pending.json::<Value>().await.unwrap(),
        json!({"pending": true})
    );
    assert_eq!(
        client
            .put(format!("{panel}/api/wake-models"))
            .json(&json!({"tag":"v1","id":"fixture"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    crate::station::capture::Detector::new(None).apply_pending_model(&app);
    for _ in 0..2 {
        let ready = result().send().await.unwrap();
        assert_eq!(ready.status(), StatusCode::OK);
        assert_eq!(
            ready
                .json::<crate::wake_catalog::Selection>()
                .await
                .unwrap(),
            expected
        );
    }
    let state: Value = client
        .get(format!("{panel}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["wake_model"]["id"], expected.id);
    task.abort();
}

#[tokio::test]
async fn panel_state_includes_declared_plugin_page_icons() {
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
    app.plugins.mock_plugin(
        "fixture",
        Process::mock_rpc(|_, _| unreachable!()),
        json!({
            "apiVersion": 1,
            "pages": [
                {"id": "main", "path": "", "title": {"en": "Timers", "ru": "Таймеры"}, "navigation": true, "icon": "timer"},
                {"id": "legacy", "path": "legacy", "title": {"en": "Legacy", "ru": "Архив"}, "navigation": true},
            ],
        }),
    );
    let (panel, task) = serve(router(app)).await;
    let state: Value = reqwest::get(format!("{panel}/api/state"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let pages = &state["plugins"][0]["pages"];
    assert_eq!(pages[0]["icon"], "timer");
    assert_eq!(pages[1]["navigation"], true);
    assert!(pages[1].get("icon").is_none());
    task.abort();
}

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
async fn open_panel_shows_resources_but_requires_a_password_for_logs() {
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
    let resources = reqwest::get(format!("{panel}/api/diagnostics/resources"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(resources["supported"], cfg!(target_os = "linux"));
    let logs = reqwest::get(format!("{panel}/api/diagnostics/logs"))
        .await
        .unwrap();
    assert_eq!(logs.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        logs.json::<Value>().await.unwrap()["error"],
        "diagnostics_password_required"
    );
    task.abort();
}

#[tokio::test]
async fn panel_serves_precompressed_assets_at_their_original_urls() {
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
    let client = reqwest::Client::builder().no_gzip().build().unwrap();
    for (path, mime, original, compressed) in [
        (
            "/",
            "text/html; charset=utf-8",
            include_bytes!("../../web/dist/index.html").as_slice(),
            include_bytes!("../../web/dist/index.html.gz").as_slice(),
        ),
        (
            "/panel.js",
            "text/javascript; charset=utf-8",
            include_bytes!("../../web/dist/panel.js").as_slice(),
            include_bytes!("../../web/dist/panel.js.gz").as_slice(),
        ),
        (
            "/panel.css",
            "text/css; charset=utf-8",
            include_bytes!("../../web/dist/panel.css").as_slice(),
            include_bytes!("../../web/dist/panel.css.gz").as_slice(),
        ),
        (
            "/plugin-runtime-v1.js",
            "text/javascript; charset=utf-8",
            include_bytes!("../../web/dist/plugin-runtime-v1.js").as_slice(),
            include_bytes!("../../web/dist/plugin-runtime-v1.js.gz").as_slice(),
        ),
        (
            "/panel-licenses.md",
            "text/plain; charset=utf-8",
            include_bytes!("../../web/dist/panel-licenses.md").as_slice(),
            include_bytes!("../../web/dist/panel-licenses.md.gz").as_slice(),
        ),
    ] {
        let response = client.get(format!("{panel}{path}")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
        if path == "/" {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        } else {
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                crate::panel_core::ASSET_CACHE_CONTROL
            );
            let tag = response.headers()[header::ETAG].to_str().unwrap();
            let cached = client
                .get(format!("{panel}{path}"))
                .header(header::IF_NONE_MATCH, format!("W/{tag}"))
                .send()
                .await
                .unwrap();
            assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(cached.headers()[header::ETAG], tag);
            assert!(cached.bytes().await.unwrap().is_empty());
        }
        assert_eq!(response.content_length(), Some(compressed.len() as u64));
        let body = response.bytes().await.unwrap();
        assert_eq!(body.as_ref(), compressed);
        let mut decoded = Vec::new();
        GzDecoder::new(body.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, original);
    }

    let response = client
        .head(format!("{panel}/panel.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    assert!(response.bytes().await.unwrap().is_empty());
    task.abort();
}

async fn gzip_text(response: reqwest::Response) -> String {
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    let body = response.bytes().await.unwrap();
    let mut text = String::new();
    GzDecoder::new(body.as_ref())
        .read_to_string(&mut text)
        .unwrap();
    text
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
    let client = reqwest::Client::builder().no_gzip().build().unwrap();
    for path in [
        "/",
        "/settings",
        "/settings/?from=bookmark",
        "/settings/activation",
        "/settings/assistant",
        "/setup",
        "/station",
        "/settings/diagnostics/",
        "/settings/panel",
        "/settings/update",
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
        assert!(gzip_text(response).await.contains("/panel.js"));
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
    assert!(gzip_text(response).await.contains("createSignal"));
    let response = client
        .get(format!("{panel}/panel-licenses.md"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    let licenses = gzip_text(response).await;
    assert!(licenses.contains("Lucide Icons and Contributors"));
    assert!(licenses.contains("Cole Bemis"));
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
    let response = client
        .get(format!("{panel}/api/core-update"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let update = response.json::<Value>().await.unwrap();
    assert_eq!(update["version"], crate::core_update::VERSION);
    assert_eq!(update["supported"], false);
    for request in [
        client.get(format!("{panel}/api/core-update/latest")),
        client
            .post(format!("{panel}/api/core-update"))
            .json(&json!({"version": "9.0.0"})),
    ] {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"],
            "core_update_password_required"
        );
    }
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

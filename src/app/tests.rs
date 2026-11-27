use super::*;
use crate::config::{Access, DEFAULT_MICROPHONE_GAIN};

fn test_app(dir: &std::path::Path) -> Arc<App> {
    App::new(
        dir.into(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
        Player::new(),
        DEFAULT_MICROPHONE_GAIN,
    )
}

#[test]
fn a_wake_probe_keeps_the_last_command_and_corrections_require_its_id() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    app.set_model(true, None);
    let original = app.start(Source::ManualTest).unwrap();
    app.finish(original, None);
    let probe = app.start(Source::WakeProbe).unwrap();
    assert_eq!(app.active().unwrap().stage, Stage::AwaitingWake);
    app.finish(probe, Some(Failure::NoWake));
    let snapshot = app.snapshot();
    assert_eq!(
        snapshot.last_voice.unwrap().report.request_id,
        Some(original)
    );
    assert_eq!(snapshot.last_probe.unwrap().request_id, Some(probe));
    assert!(app.start_correction(probe).is_err());
    let correction = app.start_correction(original).unwrap();
    assert!(app.start_correction(original).is_err());
    assert_eq!(app.active().unwrap().stage, Stage::Thinking);
    app.finish(correction, None);
    assert!(app.start_correction(original).is_err());
}

#[tokio::test]
async fn chat_failure_stays_on_its_message_without_replacing_the_last_voice_test() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    let test = app.start(Source::ManualTest).ok().unwrap();
    app.finish(test, Some(Failure::NoSpeech));

    let chat = app.start(Source::Chat).ok().unwrap();
    app.chat(chat, "first message".into()).await;
    let next_chat = app.start(Source::Chat).ok().unwrap();
    app.chat(next_chat, "second message".into()).await;

    let state = app.snapshot();
    let events = serde_json::to_value(&state.system_events).unwrap();
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["request_id"], test);
    assert_eq!(events[0]["error"], "no_speech");
    assert_eq!(events[1]["message_id"], state.messages[0].id);
    assert_eq!(events[2]["message_id"], state.messages[1].id);
    assert!(events[0]["id"].as_u64().unwrap() < state.messages[0].id);
    assert!(state.messages[0].id < events[1]["id"].as_u64().unwrap());
    assert!(events[1]["id"].as_u64().unwrap() < state.messages[1].id);
    assert_eq!(recent_chat(state.messages.iter().rev()).len(), 2);
    let last = state.last.unwrap();
    assert!(last.source == Source::Chat);
    assert_eq!(last.message_id, Some(state.messages[1].id));
    assert!(matches!(
        state.messages[0].error,
        Some(Failure::ProviderUnavailable)
    ));
    assert!(matches!(
        state.messages[1].error,
        Some(Failure::ProviderUnavailable)
    ));
    let last_test = state.last_voice_test.unwrap();
    assert!(last_test.report.source == Source::ManualTest);
    assert!(matches!(last_test.report.error, Some(Failure::NoSpeech)));
    let voice = state.last_voice.unwrap();
    assert!(voice.report.source == Source::ManualTest);
    assert!(voice.details.error_stage == Some(Stage::Recording));

    assert!(app.clear_chat().is_ok());
    let cleared = app.snapshot();
    assert!(cleared.messages.is_empty());
    assert_eq!(
        serde_json::to_value(cleared.system_events).unwrap(),
        serde_json::json!([])
    );
    assert!(cleared.last_voice_test.is_some());
}

#[test]
fn conversation_diagnostics_keep_recent_turns_and_skip_wake_probes() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    app.set_model(true, None);
    let mut latest = 0;
    for _ in 0..10 {
        latest = app.start(Source::ManualTest).unwrap();
        app.update_voice(latest, |details, _| {
            details.transcript = Some("unanswered".into())
        });
        app.finish(latest, Some(Failure::ProviderFailed));
    }
    let probe = app.start(Source::WakeProbe).unwrap();
    app.finish(probe, Some(Failure::NoWake));
    let state = app.snapshot();
    assert!(state.messages.is_empty());
    let events = serde_json::to_value(state.system_events).unwrap();
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 8);
    assert_eq!(events[0]["request_id"], latest - 7);
    assert_eq!(events[7]["request_id"], latest);
    assert_eq!(events[7]["transcript"], "unanswered");
}

#[test]
fn the_last_voice_test_ignores_regular_wake_requests_and_updates_after_a_wake_test() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    app.set_model(true, None);
    let manual = app.start(Source::ManualTest).ok().unwrap();
    app.finish(manual, None);

    let wake = app.start(Source::Wake).ok().unwrap();
    app.finish(wake, None);
    let state = app.snapshot();
    assert!(state.last.unwrap().source == Source::Wake);
    assert!(state.last_voice.unwrap().report.source == Source::Wake);
    assert!(state.last_voice_test.unwrap().report.source == Source::ManualTest);

    let test = app.start(Source::WakeTest).ok().unwrap();
    app.finish(test, Some(Failure::NoWake));
    let report = app.snapshot().last_voice_test.unwrap();
    assert!(report.report.source == Source::WakeTest);
    assert!(matches!(report.report.error, Some(Failure::NoWake)));
}

#[tokio::test]
async fn explicit_chat_tests_keep_the_model_used_when_the_request_started() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    app.plugins.mock_provider(
        crate::assistant::DEFAULT_PROVIDER,
        crate::plugins::Process::mock_rpc(|method, body| {
            assert_eq!(method, "provider.complete");
            let messages = body["messages"].as_array().unwrap();
            if messages.last().unwrap()["content"]
                .as_str()
                .unwrap()
                .starts_with("Check")
            {
                assert_eq!(messages.len(), 2);
                assert!(body["selection"]["model"].is_null());
            }
            Ok(serde_json::json!({ "message": { "role": "assistant", "content": "Hello" } }))
        }),
    );
    let regular_chat = app.start(Source::Chat).unwrap();
    app.chat(regular_chat, "An ordinary message".into()).await;
    assert!(app.snapshot().last_chat_test.is_none());

    let test = app.start_chat_test().unwrap();
    app.chat(test, "Check".into()).await;
    let state = app.snapshot();
    assert!(state.chat_test_current);
    assert!(state.last_chat_test.unwrap().chat_test.unwrap().answered);

    let test = app.start_chat_test().unwrap();
    let mut settings = app.config.request_settings().settings;
    settings.assistant.pipeline.chat.selection.model = Some("new-model".into());
    app.config.update_settings(settings).unwrap();
    app.chat(test, "Check while the setting changes".into())
        .await;
    let state = app.snapshot();
    let test = state.last_chat_test.unwrap().chat_test.unwrap();
    assert!(test.answered);
    assert!(test.model.model.is_none());
    assert!(!state.chat_test_current);
}

#[test]
fn changing_microphone_gain_requires_a_new_voice_test() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    let chat = app.start_chat_test().unwrap();
    app.finish(chat, None);
    let voice = app.start(Source::ManualTest).unwrap();
    app.finish(voice, None);
    assert!(app.snapshot().voice_test_current);

    let mut settings = app.config.request_settings().settings;
    settings.transcription_gain = Some(2.0);
    app.config.update_settings(settings).unwrap();
    let state = app.snapshot();
    assert!(!state.voice_test_current);
    assert!(state.chat_test_current);
}

#[test]
fn silence_closes_only_the_pending_follow_up_and_preserves_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    app.set_model(true, None);
    let mut detector = crate::station::capture::Detector::new(None);
    let wake = app.start(Source::Wake).unwrap();
    app.stage(wake, Stage::Playing);
    app.finish(wake, None);
    let waiting = app.active().unwrap();
    assert_eq!(waiting.stage, Stage::AwaitingSpeech);
    assert!(!app.cancel_voice(wake));
    let mut settings = app.config.request_settings().settings;
    settings.dialogue.timeout_seconds = 2;
    app.config.update_settings(settings.clone()).unwrap();
    app.runtime().active.as_mut().unwrap().started -= std::time::Duration::from_secs(2);
    for _ in 0..crate::station::capture::VAD_SAMPLES {
        detector.push_sample(0, &app);
    }
    assert!(app.active().is_none());
    assert!(!app.cancel_voice(waiting.id));
    assert_eq!(
        app.snapshot().last_voice.unwrap().report.request_id,
        Some(wake)
    );
    let next = app.start(Source::Wake).unwrap();
    app.stage(next, Stage::Playing);
    app.finish(next, None);
    let waiting = app.active().unwrap();
    assert!(app.cancel_voice(waiting.id));
    assert!(app.active().is_none());
    assert_eq!(
        app.snapshot().last_voice.unwrap().report.request_id,
        Some(next)
    );
    settings.dialogue.mode = crate::voice_core::DialogueMode::SingleTurn;
    app.config.update_settings(settings).unwrap();
    let single = app.start(Source::Wake).unwrap();
    app.stage(single, Stage::Playing);
    app.finish(single, None);
    assert!(app.active().is_none());
}

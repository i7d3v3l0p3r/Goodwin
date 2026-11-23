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
    assert!(last_test.source == Source::ManualTest);
    assert!(matches!(last_test.error, Some(Failure::NoSpeech)));

    assert!(app.clear_chat().is_ok());
    let cleared = app.snapshot();
    assert!(cleared.messages.is_empty());
    assert!(cleared.last_voice_test.is_some());
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
    assert!(state.last_voice_test.unwrap().source == Source::ManualTest);

    let test = app.start(Source::WakeTest).ok().unwrap();
    app.finish(test, Some(Failure::NoWake));
    let report = app.snapshot().last_voice_test.unwrap();
    assert!(report.source == Source::WakeTest);
    assert!(matches!(report.error, Some(Failure::NoWake)));
}

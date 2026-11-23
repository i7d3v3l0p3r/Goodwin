use std::{fs, sync::Barrier, thread};

use super::{Access, Config, ConfigStore, Settings};

#[test]
fn failed_save_keeps_the_active_settings_and_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("not-a-directory");
    fs::write(&blocked, b"file").unwrap();
    let store = ConfigStore::new(
        &blocked,
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: "Old prompt".into(),
        },
    );

    let settings = Settings {
        listening: false,
        ..Settings::default()
    };
    assert!(store.update_settings(settings).is_err());
    assert!(store.update_system_prompt("New prompt".into()).is_err());
    let request = store.request_settings();
    assert!(request.settings.listening);
    assert_eq!(request.system_prompt, "Old prompt");
}

#[test]
fn concurrent_settings_updates_keep_the_committed_preferences_and_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(
        dir.path(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            system_prompt: String::new(),
        },
    );
    let start = Barrier::new(2);
    thread::scope(|scope| {
        scope.spawn(|| {
            let settings = Settings {
                listening: false,
                ..Settings::default()
            };
            start.wait();
            store.update_settings(settings).unwrap();
        });
        scope.spawn(|| {
            start.wait();
            store.update_system_prompt("Новый характер".into()).unwrap();
        });
    });

    let saved = Config::load(dir.path()).unwrap();
    let request = store.request_settings();
    assert_eq!(request.settings.listening, saved.settings.listening);
    assert_eq!(saved.system_prompt, "Новый характер");
    assert_eq!(request.system_prompt, saved.system_prompt);
}

#[test]
fn configuration_without_provider_bindings_or_a_system_prompt_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = serde_json::to_value(Settings::default()).unwrap();
    let fields = settings.as_object_mut().unwrap();
    fields.remove("assistant");
    fields.remove("microphone_muted");
    let config = serde_json::json!({
        "access": {"mode": "open"},
        "settings": settings,
    });
    fs::write(Config::path(dir.path()), config.to_string()).unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.system_prompt.is_empty());
    assert!(!config.settings.microphone_muted);
    assert_eq!(
        config.settings.assistant.pipeline.chat.provider,
        "openrouter"
    );
    assert_eq!(
        config.settings.assistant.pipeline.transcription.provider,
        "openrouter"
    );
    assert_eq!(
        config.settings.assistant.pipeline.speech.provider,
        "openrouter"
    );
}

#[test]
fn speech_streaming_defaults_on_and_can_be_disabled_in_saved_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = serde_json::to_value(Settings::default()).unwrap();
    let pipeline = settings["assistant"]["pipeline"].as_object_mut().unwrap();
    pipeline.remove("speech_streaming");
    pipeline["speech"]["provider"] = "openai-compatible".into();
    fs::write(
        Config::path(dir.path()),
        serde_json::json!({
            "access": {"mode": "open"},
            "settings": settings,
        })
        .to_string(),
    )
    .unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.settings.assistant.pipeline.speech_streaming);

    let mut settings = config.settings.clone();
    settings.assistant.pipeline.speech_streaming = false;
    ConfigStore::new(dir.path(), config)
        .update_settings(settings)
        .unwrap();
    let saved = Config::load(dir.path()).unwrap();
    assert!(!saved.settings.assistant.pipeline.speech_streaming);
    assert_eq!(
        saved.settings.assistant.pipeline.speech.provider,
        "openai-compatible"
    );
}

#[test]
fn previous_provider_bindings_load_without_importing_openrouter_settings() {
    use serde_json::json;

    let root = tempfile::tempdir().unwrap();
    fs::write(
        Config::path(root.path()),
        json!({
            "access": {"mode": "open"},
            "settings": {
                "listening": false,
                "sensitivity": 0.8,
                "providers": {"transcription": null, "chat":"another-provider", "speech":null},
                "models": {"chat": "fixture/chat", "speech": "fixture/speech", "voice": "Fixture"}
            },
            "key": "fixture-migration-key",
            "system_prompt": "Fixture prompt"
        })
        .to_string(),
    )
    .unwrap();

    let config = Config::load(root.path()).unwrap();
    assert_eq!(
        config.settings.assistant.pipeline.transcription.provider,
        "openrouter"
    );
    assert_eq!(
        config.settings.assistant.pipeline.chat.provider,
        "another-provider"
    );
    assert_eq!(
        config.settings.assistant.pipeline.speech.provider,
        "openrouter"
    );
    assert!(!config.settings.listening);
    assert_eq!(config.settings.wake_gain, None);
    assert_eq!(config.settings.transcription_gain, None);
    assert_eq!(config.system_prompt, "Fixture prompt");
    let saved = serde_json::to_value(&config).unwrap();
    assert!(saved.get("key").is_none());
    assert!(saved["settings"].get("models").is_none());
    assert!(!root.path().join("plugins").exists());
}

#[test]
fn microphone_gains_reject_nonfinite_or_out_of_range_values_independently() {
    for invalid in [0.0, 65.0, f32::NAN, f32::INFINITY] {
        let mut settings = Settings {
            wake_gain: Some(invalid),
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        settings.wake_gain = None;
        settings.transcription_gain = Some(invalid);
        assert!(settings.validate().is_err());
    }
    let settings = Settings {
        wake_gain: Some(0.1),
        transcription_gain: Some(64.0),
        ..Settings::default()
    };
    assert!(settings.validate().is_ok());
}

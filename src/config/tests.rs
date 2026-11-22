use std::{fs, sync::Barrier, thread};

use super::{Access, Config, ConfigStore, Settings};

#[test]
fn failed_save_keeps_the_active_settings_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("not-a-directory");
    fs::write(&blocked, b"file").unwrap();
    let store = ConfigStore::new(
        &blocked,
        Config {
            access: Access::Open,
            settings: Settings::default(),
            key: "old-key".into(),
            system_prompt: "Old prompt".into(),
        },
    );

    let settings = Settings {
        listening: false,
        ..Settings::default()
    };
    assert!(
        store
            .update_settings(settings, Some("new-key".into()))
            .is_err()
    );
    assert!(store.update_system_prompt("New prompt".into()).is_err());
    let request = store.request_settings();
    assert!(request.settings.listening);
    assert_eq!(request.key, "old-key");
    assert_eq!(request.system_prompt, "Old prompt");
}

#[test]
fn concurrent_settings_updates_keep_the_committed_key_and_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(
        dir.path(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            key: "old-key".into(),
            system_prompt: String::new(),
        },
    );
    let start = Barrier::new(3);
    thread::scope(|scope| {
        scope.spawn(|| {
            start.wait();
            store
                .update_settings(Settings::default(), Some("new-key".into()))
                .unwrap();
        });
        scope.spawn(|| {
            let settings = Settings {
                listening: false,
                ..Settings::default()
            };
            start.wait();
            store.update_settings(settings, None).unwrap();
        });
        scope.spawn(|| {
            start.wait();
            store.update_system_prompt("Новый характер".into()).unwrap();
        });
    });

    let saved = Config::load(dir.path()).unwrap();
    let request = store.request_settings();
    assert_eq!(saved.key, "new-key");
    assert_eq!(request.key, saved.key);
    assert_eq!(request.settings.listening, saved.settings.listening);
    assert_eq!(saved.system_prompt, "Новый характер");
    assert_eq!(request.system_prompt, saved.system_prompt);
}

#[test]
fn configuration_without_a_system_prompt_loads_with_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = serde_json::json!({
        "access": {"mode": "open"},
        "settings": Settings::default(),
    });
    fs::write(Config::path(dir.path()), config.to_string()).unwrap();
    assert!(Config::load(dir.path()).unwrap().system_prompt.is_empty());
}

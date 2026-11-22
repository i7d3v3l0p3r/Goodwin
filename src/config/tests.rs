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
    let (settings, key) = store.request_settings();
    assert!(settings.listening);
    assert_eq!(key, "old-key");
}

#[test]
fn concurrent_settings_updates_keep_the_committed_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(
        dir.path(),
        Config {
            access: Access::Open,
            settings: Settings::default(),
            key: "old-key".into(),
        },
    );
    let start = Barrier::new(2);
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
    });

    let saved = Config::load(dir.path()).unwrap();
    let (settings, key) = store.request_settings();
    assert_eq!(saved.key, "new-key");
    assert_eq!(key, saved.key);
    assert_eq!(settings.listening, saved.settings.listening);
}

use std::{
    future::{pending, poll_fn},
    pin::{Pin, pin},
    task::Poll,
};

use tokio::io::{AsyncReadExt, duplex};

use crate::{
    app::{Source, Stage},
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    station::Player,
};

use super::*;

fn app(dir: &std::path::Path) -> Arc<App> {
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

async fn poll_pending(mut future: Pin<&mut impl Future>) {
    poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn switches_during_a_chat_without_waiting_for_or_discarding_audio() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(temp.path());
    let turn = app.start(Source::Chat).ok().unwrap();
    let (mut writer, mut reader) = duplex(8);
    let mut detector = Detector::new(None);
    let mut bytes = [0; 2];
    {
        writer.write_all(b"a").await.unwrap();
        let mut audio = pin!(with_model_updates(
            &app,
            &mut detector,
            reader.read_exact(&mut bytes)
        ));
        poll_pending(audio.as_mut()).await;

        let (ticket, _) = app.wake_models.stage_fixture("egor-standard");
        poll_pending(audio.as_mut()).await;
        assert!(matches!(
            app.wake_models.install_result(&ticket),
            Some(Ok(Some(_)))
        ));
        assert_eq!(app.active().unwrap().id, turn);
        assert_eq!(
            serde_json::to_value(app.snapshot()).unwrap()["wake_model"]["id"],
            "egor-standard"
        );

        writer.write_all(b"b").await.unwrap();
        audio.await.unwrap();
    }
    assert_eq!(&bytes, b"ab");
}

#[tokio::test]
async fn a_wake_test_keeps_its_model_until_it_starts_recording() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(temp.path());
    assert!(app.microphone_connected(&app.microphone_mute()));
    let mut detector = Detector::new(None);
    let (_, previous) = app.wake_models.stage_fixture("previous");
    detector.apply_pending_model(&app);
    let turn = app.start(Source::WakeTest).ok().unwrap();
    let mut audio = pin!(with_model_updates(&app, &mut detector, pending::<()>()));

    let (ticket, selected) = app.wake_models.stage_fixture("new-selection");
    poll_pending(audio.as_mut()).await;
    assert!(matches!(
        app.wake_models.install_result(&ticket),
        Some(Ok(None))
    ));
    assert_eq!(
        serde_json::to_value(app.snapshot()).unwrap()["wake_model"]["id"],
        previous.id
    );
    let manifest = temp.path().join("wake-model.json");
    let read_model =
        || serde_json::from_slice::<serde_json::Value>(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(read_model()["id"], previous.id);

    app.stage(turn, Stage::Recording);
    poll_pending(audio.as_mut()).await;
    assert!(
        matches!(app.wake_models.install_result(&ticket), Some(Ok(Some(model))) if model == selected)
    );
    assert_eq!(read_model()["id"], selected.id);
    assert!(app.active().unwrap().stage == Stage::Recording);
}

#[test]
fn a_failed_manifest_rename_keeps_the_previous_detector() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(temp.path());
    let mut detector = Detector::new(None);
    let (_, previous) = app.wake_models.stage_fixture("previous");
    detector.apply_pending_model(&app);
    let (ticket, _) = app.wake_models.stage_fixture("candidate");

    let manifest = temp.path().join("wake-model.json");
    std::fs::rename(&manifest, temp.path().join("backup.json")).unwrap();
    std::fs::create_dir(&manifest).unwrap();
    detector.apply_pending_model(&app);
    assert!(matches!(
        app.wake_models.install_result(&ticket),
        Some(Err(crate::wake::InstallError::Failed))
    ));
    assert_eq!(
        serde_json::to_value(app.snapshot()).unwrap()["wake_model"]["id"],
        previous.id
    );
    assert!(app.wake_models.start_install().is_ok());
}

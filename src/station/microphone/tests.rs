use std::{
    future::{pending, poll_fn},
    pin::{Pin, pin},
    task::Poll,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::io::{AsyncReadExt, duplex};

use crate::{
    app::{Source, Stage},
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings},
    station::Player,
    wake::WakeModel,
    wake_catalog::Selection,
};

use super::*;

fn queue_model(app: &App) -> tokio::sync::oneshot::Receiver<()> {
    // A small ONNX graph with a single Identity node from chunk to score.
    let encoded = concat!(
        "CAg6VwoYCgVjaHVuaxIFc2NvcmUiCElkZW50aXR5EgdmaXh0dXJl",
        "WhgKBWNodW5rEg8KDQgBEgkKAggBCgMIgApiGAoFc2NvcmUSDwoNCAESCQoCCAEKAwiACkICEA0=",
    );
    let path = app.dir.join("fixture.onnx");
    std::fs::write(&path, STANDARD.decode(encoded).unwrap()).unwrap();
    app.wake_models.queue_model(
        Selection {
            id: "egor-standard".into(),
            wake_word: "Егор".into(),
            tier: "standard".into(),
            sha256: "0".repeat(64),
            runtime: None,
            score_threshold: None,
        },
        WakeModel::load(&path).unwrap(),
    )
}

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

        let mut activated = queue_model(&app);
        poll_pending(audio.as_mut()).await;
        assert!(activated.try_recv().is_ok());
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
    app.set_model(true, None);
    let turn = app.start(Source::WakeTest).ok().unwrap();
    let mut detector = Detector::new(None);
    let mut audio = pin!(with_model_updates(&app, &mut detector, pending::<()>()));

    let mut activated = queue_model(&app);
    poll_pending(audio.as_mut()).await;
    assert!(matches!(
        activated.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(serde_json::to_value(app.snapshot()).unwrap()["wake_model"].is_null());

    app.stage(turn, Stage::Recording);
    poll_pending(audio.as_mut()).await;
    assert!(activated.try_recv().is_ok());
    assert!(app.active().unwrap().stage == Stage::Recording);
}

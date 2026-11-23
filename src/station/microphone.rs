use std::{
    future::Future,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use prost::Message;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::watch,
    time::{sleep, timeout},
};

use crate::app::App;

use super::{
    capture::Detector,
    protocol::{FRAME_TIMEOUT, Framing, capture_channel, receive},
};

#[cfg(test)]
mod tests;

pub async fn run_microphone(app: Arc<App>, address: SocketAddr) {
    let loader = Arc::clone(&app);
    let model = match tokio::task::spawn_blocking(move || loader.wake_models.initial_model()).await
    {
        Ok(Ok((model, selection))) => {
            app.set_model(true, selection);
            Some(model)
        }
        Ok(Err(error)) => {
            eprintln!("Wake model unavailable: {error:#}");
            None
        }
        Err(error) => {
            eprintln!("Wake model loader failed: {error}");
            None
        }
    };

    let mut detector = Detector::new(model);
    let mut framing = Framing::Typed;
    let mut mute = app.microphone_mute();
    loop {
        if *mute.borrow_and_update() {
            with_model_updates(&app, &mut detector, mute.changed())
                .await
                .ok();
            continue;
        }
        let connection_mute = mute.clone();
        let connection_failed = tokio::select! {
            biased;
            _ = mute.changed() => false,
            result = stream(&app, address, framing, &mut detector, &connection_mute) => result.is_err(),
        };
        app.microphone_disconnected();
        detector.clear();
        if connection_failed {
            framing = framing.alternate();
            with_model_updates(&app, &mut detector, async {
                tokio::select! {
                    _ = mute.changed() => {},
                    () = sleep(Duration::from_secs(1)) => {},
                }
            })
            .await;
        }
    }
}

async fn stream(
    app: &Arc<App>,
    address: SocketAddr,
    framing: Framing,
    detector: &mut Detector,
    mute: &watch::Receiver<bool>,
) -> Result<()> {
    let mut socket = with_model_updates(app, detector, async {
        timeout(FRAME_TIMEOUT, async {
            let mut socket = TcpStream::connect(address).await?;
            let request = framing.subscription().encode_to_vec();
            socket.write_u32_le(request.len() as u32).await?;
            socket.write_all(&request).await?;
            Ok::<_, std::io::Error>(socket)
        })
        .await?
    })
    .await?;
    let mut buffer = Vec::new();
    let mut audio_at = Instant::now();

    loop {
        let channels = with_model_updates(app, detector, receive(&mut socket, &mut buffer)).await?;
        let Some(audio) = channels.as_ref().and_then(capture_channel) else {
            ensure!(audio_at.elapsed() < FRAME_TIMEOUT, "No VQE audio");
            continue;
        };
        audio_at = Instant::now();
        if !app.microphone_connected(mute) {
            return Ok(());
        }
        for &bytes in audio.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(bytes);
            detector.push_sample(sample, app);
        }
    }
}

async fn with_model_updates<F: Future>(app: &App, detector: &mut Detector, audio: F) -> F::Output {
    // An update must leave partial socket reads in progress.
    tokio::pin!(audio);
    loop {
        tokio::select! {
            biased;
            () = app.wake_models.changed.notified() => detector.apply_pending_model(app),
            result = &mut audio => return result,
        }
    }
}

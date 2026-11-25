use std::{
    future::Future,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use goodwin_error::{Context, Result, ensure};
use prost::Message;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::watch,
    time::{sleep, timeout},
};

use crate::{app::App, station::capture::Detector};

use super::protocol::{FRAME_TIMEOUT, Framing, capture_channel, receive};

#[cfg(test)]
mod tests;

pub async fn run_microphone(app: Arc<App>, address: SocketAddr) {
    let loader = Arc::clone(&app);
    let (model, selection) =
        match tokio::task::spawn_blocking(move || loader.wake_models.initial_model()).await {
            Ok(Ok((model, selection))) => {
                let id = selection
                    .as_ref()
                    .map_or("local", |model| model.id.as_str());
                log::info!("Wake model loaded id={id}");
                app.set_model(true, selection.clone());
                (Some(model), selection)
            }
            Ok(Err(error)) => {
                log::warn!("Wake model unavailable: {error:#}");
                (None, None)
            }
            Err(error) => {
                log::error!("Wake model loader failed: {error}");
                (None, None)
            }
        };

    let mut detector = Detector::new(model).with_selection(selection.as_ref());
    let mut framing = Framing::Typed;
    let mut mute = app.microphone_mute();
    let mut outage_reported = false;
    loop {
        if *mute.borrow_and_update() {
            with_model_updates(&app, &mut detector, mute.changed())
                .await
                .ok();
            continue;
        }
        let connection_mute = mute.clone();
        let connection = tokio::select! {
            biased;
            _ = mute.changed() => Ok(()),
            result = stream(&app, address, framing, &mut detector, &connection_mute, &mut outage_reported) => result,
        };
        app.microphone_disconnected();
        detector.clear(&app);
        if let Err(error) = connection {
            if !outage_reported {
                log::warn!("Microphone stream unavailable: {error:#}");
                outage_reported = true;
            }
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
    outage_reported: &mut bool,
) -> Result<()> {
    let mut socket = with_model_updates(app, detector, async {
        timeout(FRAME_TIMEOUT, async {
            let mut socket = TcpStream::connect(address)
                .await
                .context("Connect to VQE")?;
            let request = framing.subscription().encode_to_vec();
            socket.write_u32_le(request.len() as u32).await?;
            socket.write_all(&request).await?;
            Ok::<_, goodwin_error::Error>(socket)
        })
        .await
        .context("Subscribe to VQE audio timed out")?
    })
    .await?;
    let mut buffer = Vec::new();
    let mut audio_at = Instant::now();
    let mut received_audio = false;

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
        if !received_audio {
            log::info!("Microphone audio connected");
            received_audio = true;
            *outage_reported = false;
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

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use prost::Message;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    time::{sleep, timeout},
};

use crate::{
    app::{App, Failure, Stage},
    wake::WakeModel,
};

use super::{
    capture::Detector,
    protocol::{FRAME_TIMEOUT, Framing, capture_channel, receive},
};

pub struct MicrophoneOptions {
    pub address: SocketAddr,
    pub gain: f32,
}

pub async fn run_microphone(app: Arc<App>, options: MicrophoneOptions) {
    let model_path = std::env::var_os("GOODWIN_WAKE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| app.dir.join("goodwin.onnx"));
    let model = tokio::task::spawn_blocking(move || load_model(&model_path))
        .await
        .ok()
        .flatten();
    app.set_model(model.is_some());

    let mut detector = Detector::new(model);
    let mut framing = Framing::Typed;
    loop {
        let result = stream(&app, &options, framing, &mut detector).await;
        app.set_microphone(false);
        if let Some((id, _, Stage::AwaitingWake | Stage::Recording, _, _)) = app.active() {
            app.finish(id, Some(Failure::Microphone));
        }
        detector.clear();
        if result.is_err() {
            framing = framing.alternate();
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn load_model(path: &Path) -> Option<WakeModel> {
    if !path.is_file() {
        eprintln!(
            "Wake model is missing at {}. Add the model and restart.",
            path.display()
        );
        return None;
    }
    match WakeModel::load(path) {
        Ok(model) => {
            println!("Loaded {} wake model from {}", model.kind(), path.display());
            Some(model)
        }
        Err(error) => {
            eprintln!(
                "Could not load the wake model {}: {error:#}",
                path.display()
            );
            None
        }
    }
}

async fn stream(
    app: &Arc<App>,
    options: &MicrophoneOptions,
    framing: Framing,
    detector: &mut Detector,
) -> Result<()> {
    let mut socket = timeout(FRAME_TIMEOUT, TcpStream::connect(options.address)).await??;
    let request = framing.subscription().encode_to_vec();
    socket.write_u32_le(request.len() as u32).await?;
    socket.write_all(&request).await?;
    let mut buffer = Vec::new();
    let mut audio_at = Instant::now();

    loop {
        let channels = receive(&mut socket, &mut buffer).await?;
        let Some(audio) = channels.as_ref().and_then(capture_channel) else {
            ensure!(audio_at.elapsed() < FRAME_TIMEOUT, "No VQE audio");
            continue;
        };
        audio_at = Instant::now();
        app.set_microphone(true);
        for &bytes in audio.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(bytes);
            let sample = (f32::from(sample) * options.gain)
                .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
            detector.push_sample(sample, app);
        }
    }
}

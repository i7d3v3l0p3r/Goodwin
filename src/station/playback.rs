use std::{ffi::OsString, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

use crate::{
    app::Failure,
    providers::{Provider, SpeechStream},
    wav::PcmAudio,
};

use super::indicators::Indicators;

#[derive(Clone)]
pub struct Player {
    program: OsString,
    device: OsString,
    queue: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    sink: Option<TestSink>,
}

#[cfg(test)]
pub(super) type TestSink = Arc<std::sync::Mutex<Vec<Vec<u8>>>>;

impl Player {
    pub fn new() -> Self {
        Self {
            program: std::env::var_os("GOODWIN_APLAY").unwrap_or_else(|| "aplay".into()),
            device: std::env::var_os("GOODWIN_PLAYBACK_DEVICE")
                .unwrap_or_else(|| "speakers".into()),
            queue: Arc::default(),
            #[cfg(test)]
            sink: None,
        }
    }

    #[cfg(test)]
    pub(super) fn recording_to(sink: TestSink) -> Self {
        Self {
            sink: Some(sink),
            ..Self::new()
        }
    }

    pub async fn play(&self, wave: &[u8]) -> Result<()> {
        self.play_wave(wave, None).await
    }

    pub(crate) async fn play_speech(&self, wave: &[u8], indicators: &Indicators) -> Result<()> {
        self.play_wave(wave, Some(indicators)).await
    }

    async fn play_wave(&self, wave: &[u8], speech: Option<&Indicators>) -> Result<()> {
        let _turn = self.queue.lock().await;
        #[cfg(test)]
        if let Some(sink) = &self.sink {
            sink.lock().unwrap().push(wave.to_vec());
            return Ok(());
        }
        let speech = speech
            .and_then(|indicators| indicators.envelope(wave).map(|levels| (indicators, levels)));
        let mut child = self.command().spawn().context("Start aplay")?;
        timeout(Duration::from_secs(180), async {
            let mut input = child.stdin.take().context("Open playback input")?;
            if let Some((indicators, levels)) = speech {
                indicators.speak(levels);
            }
            input.write_all(wave).await?;
            drop(input);
            let status = child.wait().await.context("Wait for aplay")?;
            ensure!(status.success(), "aplay failed: {status}");
            Ok(())
        })
        .await
        .context("Playback timed out")?
    }

    pub(crate) async fn play_stream(
        &self,
        first: PcmAudio,
        stream: &mut SpeechStream,
        indicators: &Indicators,
    ) -> Result<(), Failure> {
        let _turn = self.queue.lock().await;
        let mut child = self
            .command()
            .args(["-t", "raw", "-f", "S16_LE", "-r"])
            .arg(first.sample_rate.to_string())
            .arg("-c")
            .arg(first.channels.to_string())
            .args(["--buffer-time=100000", "--period-time=20000"])
            .spawn()
            .context("Start aplay")
            .map_err(playback_failure)?;
        timeout(Duration::from_secs(180), async {
            let mut input = child
                .stdin
                .take()
                .context("Open playback input")
                .map_err(playback_failure)?;
            let mut envelope = indicators.begin_stream(&first);
            let streamed = async {
                input
                    .write_all(&first.data)
                    .await
                    .context("Write speech PCM")
                    .map_err(playback_failure)?;
                while let Some(audio) = stream.next().await.map_err(Provider::failure)? {
                    if let Some(envelope) = &mut envelope {
                        indicators.append_levels(envelope.push(&audio.data));
                    }
                    input
                        .write_all(&audio.data)
                        .await
                        .context("Write speech PCM")
                        .map_err(playback_failure)?;
                }
                Ok(())
            }
            .await;
            if let Some(level) = envelope.and_then(|mut envelope| envelope.finish()) {
                indicators.append_levels(vec![level]);
            }
            drop(input);
            let status = child.wait().await;
            streamed?;
            let status = status.context("Wait for aplay").map_err(playback_failure)?;
            if !status.success() {
                return Err(playback_failure(anyhow::anyhow!("aplay failed: {status}")));
            }
            Ok(())
        })
        .await
        .context("Playback timed out")
        .map_err(playback_failure)?
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .arg("-q")
            .arg("-D")
            .arg(&self.device)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }
}

pub(crate) fn playback_failure(error: anyhow::Error) -> Failure {
    log::warn!("Speech playback: {error:#}");
    Failure::Playback
}

pub(crate) fn timer_signal() -> Vec<u8> {
    use crate::wav::PcmAudio;
    let sample_rate = 16_000;
    let count = sample_rate / 3;
    let data = (0..count)
        .flat_map(|index| {
            let time = index as f32 / sample_rate as f32;
            let envelope = (index.min(count - index) as f32 / 320.0).min(1.0);
            let angle = std::f32::consts::TAU * time;
            let sample =
                envelope * ((angle * 660.0).sin() * 5000.0 + (angle * 880.0).sin() * 2500.0);
            (sample as i16).to_le_bytes()
        })
        .collect();
    let mut wave = Vec::new();
    PcmAudio {
        data,
        sample_rate,
        channels: 1,
    }
    .write_wav(&mut wave)
    .expect("valid timer signal");
    wave
}

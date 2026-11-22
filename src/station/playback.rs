use std::{ffi::OsString, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

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
        let _turn = self.queue.lock().await;
        #[cfg(test)]
        if let Some(sink) = &self.sink {
            sink.lock().unwrap().push(wave.to_vec());
            return Ok(());
        }
        let mut child = Command::new(&self.program)
            .arg("-q")
            .arg("-D")
            .arg(&self.device)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Start aplay")?;
        timeout(Duration::from_secs(180), async {
            let mut input = child.stdin.take().context("Open playback input")?;
            input.write_all(wave).await?;
            drop(input);
            ensure!(child.wait().await?.success(), "aplay failed");
            Ok(())
        })
        .await
        .context("Playback timed out")?
    }
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

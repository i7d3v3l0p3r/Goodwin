use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use goodwin_plugin::{audio::PcmAudio, wake::Model};
use serde::{Deserialize, Serialize};

use crate::files;

pub const LIMIT: usize = 1024;
pub const WINDOW_FRAMES: usize = 32;
pub const FRAME_SAMPLES: usize = 1280;
pub const SAMPLE_RATE: u32 = 16_000;
const WAV_LIMIT: usize = 44 + WINDOW_FRAMES * FRAME_SAMPLES * 2;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    pub model: Model,
    pub captured_at_ms: u64,
    pub sample_rate: u32,
    pub frame_samples: usize,
    pub gain: f32,
    pub threshold: f32,
    pub scores: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    Pending,
    Positive,
    Negative,
}

impl Label {
    const ALL: [Self; 3] = [Self::Pending, Self::Positive, Self::Negative];

    fn directory(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: u64,
    pub label: Label,
    pub info: ClipInfo,
    pub peak: f32,
    #[serde(skip)]
    wav_offset: u64,
}

pub struct Store {
    root: PathBuf,
    pub clips: BTreeMap<u64, Clip>,
    last_id: u64,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let mut store = Self {
            root: root.into(),
            clips: BTreeMap::new(),
            last_id: 0,
        };
        for label in Label::ALL {
            let directory = root.join(label.directory());
            for name in files::list(&directory)? {
                let path = directory.join(&name);
                if name.starts_with(".capture-") {
                    files::remove(&path)?;
                    continue;
                }
                let Some(id) = name
                    .strip_suffix(".clip")
                    .and_then(|id| id.parse::<u64>().ok())
                else {
                    continue;
                };
                let bytes = files::read(&path, 0, 8192, false)?;
                let header_end = bytes
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .ok_or_else(|| anyhow::anyhow!("Incomplete wake sample header"))?
                    + 1;
                let info = serde_json::from_slice(&bytes[..header_end])?;
                ensure!(
                    store
                        .clips
                        .insert(id, make_clip(id, label, info, header_end as u64))
                        .is_none(),
                    "Duplicate wake sample id"
                );
                store.last_id = store.last_id.max(id);
            }
        }
        Ok(store)
    }

    pub fn save(&mut self, info: ClipInfo, pcm: &[u8]) -> Result<()> {
        let frames = info.scores.len();
        ensure!(
            self.clips.len() < LIMIT
                && (1..=WINDOW_FRAMES).contains(&frames)
                && info.sample_rate == SAMPLE_RATE
                && info.frame_samples == FRAME_SAMPLES
                && pcm.len() == frames * FRAME_SAMPLES * 2,
            "Invalid wake sample",
        );
        let id = self.last_id.saturating_add(1).max(info.captured_at_ms);
        let label = Label::Pending;
        let mut header = serde_json::to_vec(&info)?;
        header.push(b'\n');
        ensure!(header.len() <= 8192, "Wake sample metadata is too large");
        let offset = header.len() as u64;
        PcmAudio {
            data: pcm.to_vec(),
            sample_rate: SAMPLE_RATE,
            channels: 1,
        }
        .write_wav(&mut header)?;
        files::write(&self.path(label, id), &header, false)?;
        self.clips.insert(id, make_clip(id, label, info, offset));
        self.last_id = id;
        Ok(())
    }

    pub fn wav(&self, clip: &Clip) -> Result<Vec<u8>> {
        let wav = files::read(
            &self.path(clip.label, clip.id),
            clip.wav_offset,
            WAV_LIMIT + 1,
            false,
        )?;
        ensure!(wav.len() <= WAV_LIMIT, "Wake sample is too large");
        Ok(wav)
    }

    pub fn label(&mut self, id: u64, label: Label) -> Result<()> {
        let old_label = self.clips[&id].label;
        if old_label != label {
            files::rename(&self.path(old_label, id), &self.path(label, id))?;
            self.clips.get_mut(&id).expect("known clip").label = label;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Result<()> {
        files::remove(&self.path(self.clips[&id].label, id))?;
        self.clips.remove(&id);
        Ok(())
    }

    pub fn remove_all(&mut self) -> Result<()> {
        while let Some((&id, _)) = self.clips.first_key_value() {
            self.remove(id)?;
        }
        Ok(())
    }

    fn path(&self, label: Label, id: u64) -> PathBuf {
        self.root.join(label.directory()).join(format!("{id}.clip"))
    }
}

fn make_clip(id: u64, label: Label, info: ClipInfo, wav_offset: u64) -> Clip {
    let peak = info.scores.iter().copied().reduce(f32::max).unwrap_or(0.0);
    Clip {
        id,
        label,
        info,
        peak,
        wav_offset,
    }
}

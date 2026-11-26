use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::Path,
    time::Instant,
};

use goodwin_error::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    wake::{FRAME_SAMPLES, SAMPLE_RATE, WakeModel},
    wake_catalog::Selection,
};

const FRAME_MS: u64 = 1000 * FRAME_SAMPLES as u64 / SAMPLE_RATE as u64;
const DEBOUNCE_FRAMES: u64 = 2000 / FRAME_MS;

#[derive(Deserialize)]
struct Clip {
    category: String,
    frames: u64,
    duration_ms: u64,
    window_ms: Option<[u64; 2]>,
}

#[derive(Default, Serialize)]
struct Counts {
    clips: u64,
    hits: u64,
    early_triggers: u64,
    late_triggers: u64,
    alarm_clips: u64,
    alarms: u64,
    duration_ms: u64,
}

#[derive(Serialize)]
struct OperatingPoint {
    threshold: f32,
    categories: BTreeMap<String, Counts>,
}

#[derive(Serialize)]
struct Memory {
    rss_kib: u64,
    peak_rss_kib: u64,
}

fn memory() -> Result<Memory> {
    let status = fs::read_to_string("/proc/self/status")?;
    let field = |prefix: &str| -> Result<u64> {
        Ok(status
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|value| value.split_whitespace().next())
            .context("Missing process memory statistic")?
            .parse()?)
    };
    Ok(Memory {
        rss_kib: field("VmRSS:")?,
        peak_rss_kib: field("VmHWM:")?,
    })
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let [model_path, metadata, mode] = args else {
        goodwin_error::bail!(
            "Usage: goodwin wake-bench <model> <metadata.json> <quiet|corpus-dir>"
        );
    };
    let model_path = Path::new(model_path);
    let selection: Selection = goodwin_json::from_reader(File::open(metadata)?)?;
    selection.validate()?;
    let before = memory()?;
    let start = Instant::now();
    let mut model = WakeModel::load_selected(model_path, &selection)?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let loaded = memory()?;
    for _ in 0..25 {
        model.predict(&[0; FRAME_SAMPLES])?;
    }
    let warm = memory()?;
    let model_bytes = fs::metadata(model_path)?.len();
    let default = model.threshold(0.5);
    let catalog = selection.score_threshold.unwrap_or(default);
    let mut times = Vec::new();
    let results = if mode == "quiet" {
        for _ in 0..200 {
            let start = Instant::now();
            std::hint::black_box(model.predict(&[0; FRAME_SAMPLES])?);
            times.push(start.elapsed().as_nanos() as u64);
        }
        None
    } else {
        Some(corpus(
            &mut model,
            Path::new(mode),
            [default, catalog],
            &mut times,
        )?)
    };
    let scored = memory()?;
    #[cfg(all(feature = "hotpath-alloc", feature = "wake-tflite"))]
    if let Some([arena_capacity, arena_used, state_copy]) = model.tflite_buffers() {
        eprintln!(
            "{}",
            json!({
                "tflite_arena_capacity_bytes": arena_capacity,
                "tflite_arena_used_bytes": arena_used,
                "tflite_state_copy_bytes": state_copy,
            })
        );
    }
    times.sort_unstable();
    ensure!(!times.is_empty(), "No inference frames");
    println!(
        "{}",
        json!({
            "id": selection.id,
            "sha256": selection.sha256,
            "runtime": selection.runtime,
            "model_bytes": model_bytes,
            "load_ms": load_ms,
            "frame_ms": FRAME_MS,
            "frames": times.len(),
            "predict_p50_ms": times[(times.len() - 1) / 2] as f64 / 1_000_000.0,
            "predict_p95_ms": times[(times.len() * 95).div_ceil(100) - 1] as f64 / 1_000_000.0,
            "memory": { "before": before, "loaded": loaded, "warm": warm, "scored": scored },
            "results": results,
        })
    );
    Ok(())
}

fn corpus(
    model: &mut WakeModel,
    dir: &Path,
    thresholds: [f32; 2],
    times: &mut Vec<u64>,
) -> Result<[OperatingPoint; 2]> {
    let manifest = BufReader::new(File::open(dir.join("clips.jsonl"))?);
    let mut pcm = BufReader::new(File::open(dir.join("clips.pcm"))?);
    let mut results = thresholds.map(|threshold| OperatingPoint {
        threshold,
        categories: BTreeMap::new(),
    });
    let mut bytes = [0; 2 * FRAME_SAMPLES];
    let mut samples = [0; FRAME_SAMPLES];
    for line in manifest.lines() {
        let clip: Clip = goodwin_json::from_str(&line?)?;
        ensure!(clip.frames != 0, "Empty clip");
        model.reset()?;
        let mut first = [None; 2];
        let mut last = [None; 2];
        let mut alarms = [0; 2];
        for index in 1..=clip.frames {
            pcm.read_exact(&mut bytes)?;
            for (sample, pair) in samples.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *sample = i16::from_le_bytes(*pair);
            }
            let start = Instant::now();
            let score = model.predict(&samples)?;
            times.push(start.elapsed().as_nanos() as u64);
            for (i, threshold) in thresholds.into_iter().enumerate() {
                if score >= threshold
                    && last[i].is_none_or(|previous| index - previous >= DEBOUNCE_FRAMES)
                {
                    first[i].get_or_insert(index * FRAME_MS);
                    last[i] = Some(index);
                    alarms[i] += 1;
                }
            }
        }
        for (i, result) in results.iter_mut().enumerate() {
            let counts = result.categories.entry(clip.category.clone()).or_default();
            counts.clips += 1;
            counts.duration_ms += clip.duration_ms;
            if let Some([start, end]) = clip.window_ms {
                if let Some(trigger) = first[i] {
                    counts.hits += u64::from((start..=end).contains(&trigger));
                    counts.early_triggers += u64::from(trigger < start);
                    counts.late_triggers += u64::from(trigger > end);
                }
            } else {
                counts.alarm_clips += u64::from(alarms[i] > 0);
                counts.alarms += alarms[i];
            }
        }
    }
    ensure!(pcm.fill_buf()?.is_empty(), "PCM contains unlisted frames");
    Ok(results)
}

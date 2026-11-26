"""Validate WAVs and export the corpus as LiveKit WakeWord Bench Parquet shards."""

from __future__ import annotations

import argparse
import collections
import csv
import io
import json
import math
import re
from pathlib import Path

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq
import soundfile as sf

from wakeword_dataset import read_json, write_json
from wakeword_manifest import RATE, load_corpus, verified_pcm


def parquet_schema():
    fields = {
        **dict.fromkeys(("id", "split", "category", "wake_word", "text", "text_form", "phrase",
                         "provider", "model", "voice_id", "voice_name", "accent", "gender",
                         "noise_bucket", "noise_type", "noise_source", "noise_category",
                         "noise_file", "rir_source", "rir_file", "rir_category", "interferer",
                         "noise2_category", "eq", "bandlimit", "codec", "augmentations",
                         "tts_key", "phonemes", "tts_input"), pa.string()),
        **dict.fromkeys(("label", "adversarial_distance", "n_talkers"), pa.int32()),
        **dict.fromkeys(("held_out_voice", "reverb", "party", "clipping"), pa.bool_()),
        **dict.fromkeys(("speed", "snr_db", "sir_db", "level_dbfs", "onset_s", "offset_s",
                         "phrase_onset_s", "phrase_offset_s", "duration_s"), pa.float32()),
    }
    features = {
        name: {"dtype": "float32" if pa.types.is_float32(kind) else str(kind), "_type": "Value"}
        for name, kind in fields.items()
    }
    fields["audio"] = pa.struct([("bytes", pa.binary()), ("path", pa.string())])
    features["audio"] = {"_type": "Audio", "sampling_rate": None}
    return pa.schema(list(fields.items()), metadata={
        b"huggingface": json.dumps({"info": {"features": features}}).encode(),
    })


def parquet_row(corpus, recording, schema):
    row = recording.meta
    audio = np.frombuffer(verified_pcm(corpus, recording), dtype="<i2").astype(np.float32) / 32768
    flac = io.BytesIO()
    sf.write(flac, audio, RATE, format="FLAC", subtype="PCM_16")
    active = np.flatnonzero(np.abs(audio) > max(0.003, np.max(np.abs(audio)) * 0.02))
    onset = float(active[0] / RATE) if len(active) else 0.0
    offset = float((active[-1] + 1) / RATE) if len(active) else row["duration_s"]
    background = row.get("background_type") or ""
    voice = row.get("voice") or ""
    model = row.get("model") or ""
    category = row["category"]
    meta = {
        **row, "wake_word": "Гудвин", "text": row.get("text") or "",
        "phrase": "Гудвин" if row["label"] else row["text"].split()[0] if category == "adversarial" else "",
        "provider": row.get("provider") or (model.split("/")[0] if model else "fleurs" if background == "speech" else "musan"),
        "model": model, "voice_id": row.get("group", "") if model else "", "voice_name": voice,
        "held_out_voice": bool(model and row["split"] == "test"),
        "noise_bucket": "clean", "noise_type": background,
        "noise_category": background, "noise_source": row.get("source", ""),
        "noise_file": row.get("source_recording", ""),
        "speed": 1.0, "adversarial_distance": -1, "codec": "none", "bandlimit": "none",
        "text_form": "isolated" if row.get("text", "").strip("!.?").casefold() == "гудвин" else
        "carrier_prefix" if row["label"] else "transcript" if category == "real_speech" else "",
        "level_dbfs": 20 * math.log10(max(1e-8, float(np.sqrt(np.mean(audio**2))))),
        "onset_s": onset, "offset_s": offset,
        "tts_key": row["id"] if model else "", "tts_input": row.get("text", "") if model else "",
        "audio": {"bytes": flac.getvalue(), "path": f"{row['id']}.flac"},
    }
    values = {}
    for field in schema:
        if field.name in meta:
            values[field.name] = meta[field.name]
        elif pa.types.is_string(field.type):
            values[field.name] = ""
        elif pa.types.is_boolean(field.type):
            values[field.name] = False
        elif pa.types.is_floating(field.type):
            values[field.name] = math.nan
        elif pa.types.is_integer(field.type):
            values[field.name] = 0
    return values


def export(root):
    corpus = load_corpus(root)
    rows = [recording.meta for recording in corpus.recordings]
    expected = {str(row["path"]) for row in rows}
    actual = {path.relative_to(root).as_posix()
              for category in ("positive", "negative", "background")
              for path in (root / category).rglob("*.wav")}
    if actual != expected or len(expected) != len(rows):
        raise ValueError(f"Manifest mismatch: missing={len(expected - actual)}, extra={len(actual - expected)}")
    clean = {split: items for split, items in corpus.splits.items() if items}
    schema = parquet_schema()
    target = root / "data"
    target.mkdir(exist_ok=True)
    shards = {}
    for split, items in sorted(clean.items()):
        for index in range(0, len(items), 250):
            batch = items[index:index + 250]
            destination = target / f"{split}-{index // 250:05d}.parquet"
            temporary = destination.with_suffix(".parquet.part")
            pq.write_table(pa.Table.from_pylist([parquet_row(corpus, clip, schema) for clip in batch], schema=schema),
                           temporary, row_group_size=100)
            temporary.replace(destination)
            count = pq.ParquetFile(destination).metadata.num_rows
            if count != len(batch):
                raise ValueError(f"Parquet row count differs: {destination}")
            shards[destination.name] = count
    verification = root / "verification.json"
    if verification.exists():
        stale = set(read_json(verification).get("parquet_shards", {})) - shards.keys()
        for name in stale:
            if re.fullmatch(r"(train|validation|test)-[0-9]{5}\.parquet", name):
                (target / name).unlink(missing_ok=True)
    counts = collections.Counter(row["category"] for row in rows)
    pairs = collections.Counter((row.get("voice_id"), row["label"]) for row in rows if row.get("voice_id"))
    coverage_path = root / "coverage.json"
    coverage = read_json(coverage_path) if coverage_path.exists() else []
    with (root / "coverage.csv").open("w", encoding="utf-8-sig", newline="") as stream:
        columns = ["model", "voice", "status", "reason", "positive", "negative"]
        report_csv = csv.DictWriter(stream, fieldnames=columns)
        report_csv.writeheader()
        for voice in coverage:
            report_csv.writerow({
                **{key: voice.get(key) for key in columns[:4]},
                "positive": pairs[voice["id"], 1], "negative": pairs[voice["id"], 0],
            })
    missing_classes = [
        {"model": voice["model"], "voice": voice["voice"], "voice_id": voice["id"],
         "positive": pairs[voice["id"], 1], "negative": pairs[voice["id"], 0]}
        for voice in coverage if voice["status"] == "accepted"
        and not (pairs[voice["id"], 1] and pairs[voice["id"], 0])
    ]
    report = {
        "wav_count": len(rows), "categories": dict(counts),
        "split_sizes": {split: len(items) for split, items in sorted(clean.items())},
        "passing_voices_missing_a_class": missing_classes,
        "duplicate_or_conflicting_rows": corpus.duplicates, "parquet_shards": shards,
    }
    write_json(verification, report)
    print(f"Checked {len(rows)} WAV files. Exported {sum(shards.values())} distinct clips to {len(shards)} shards.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("datasets/goodwin"))
    export(parser.parse_args().root)

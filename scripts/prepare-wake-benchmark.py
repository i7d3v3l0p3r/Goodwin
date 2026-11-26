"""Pack a held-out 16 kHz PCM corpus and matching catalog models for Goodwin's wake benchmark."""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import shutil

from wakeword_manifest import RATE, SPLITS, load_corpus, verified_pcm

FRAME = 1280
DEFAULT_MODELS = ("goodwin-standard", "goodwin-micro", "goodwin-micro-int8")


def pack_corpus(dataset, split, output):
    corpus = load_corpus(dataset)
    clips = corpus.splits[split]
    if {clip.meta["label"] for clip in clips} != {0, 1}:
        raise ValueError(f"Split {split} must contain positive and negative clips: {dataset}")
    counts = Counter()
    duration_samples = 0
    with (output / "clips.jsonl").open("w", encoding="utf-8", newline="\n") as manifest, (
        output / "clips.pcm"
    ).open("wb") as pcm:
        for recording in clips:
            clip = recording.meta
            data = verified_pcm(corpus, recording)
            samples = len(data) // 2
            window = None
            minimum_samples = samples
            category = clip["category"]
            if clip["label"] == 1:
                onset, offset = clip["phrase_onset_s"], clip["phrase_offset_s"]
                window = [math.ceil(max(onset, offset - 0.25) * 1_000),
                          math.floor((offset + 0.5) * 1_000)]
                minimum_samples = max(samples, math.ceil((offset + 0.5) * RATE))
            frames = (minimum_samples + FRAME - 1) // FRAME
            pcm.write(data)
            pcm.write(bytes((frames * FRAME - samples) * 2))
            manifest.write(json.dumps(dict(category=category, frames=frames,
                                          duration_ms=round(samples / RATE * 1_000),
                                          window_ms=window)) + "\n")
            counts[category] += 1
            duration_samples += samples
    return dict(split=split, counts=dict(counts), duration_seconds=duration_samples / RATE)


def copy_models(catalog, models, output):
    selections = {model["id"]: model for model in json.loads(catalog.read_text(encoding="utf-8"))["models"]}
    for model_id in models:
        selection = selections[model_id]
        source = catalog.parent / selection["filename"]
        if hashlib.sha256(source.read_bytes()).hexdigest() != selection["sha256"]:
            raise ValueError(f"Digest mismatch: {source}")
        shutil.copyfile(source, output / source.name)
        (output / f"{model_id}.json").write_text(
            json.dumps(selection, ensure_ascii=False) + "\n", encoding="utf-8"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path, help="Directory containing manifest.jsonl and its audio files")
    parser.add_argument("catalog", type=Path, help="Release metadata.json next to the model files")
    parser.add_argument("output", type=Path)
    parser.add_argument("--split", choices=SPLITS, default="test")
    parser.add_argument("--model", action="append", help="Catalog model ID, repeat for every runtime")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    copy_models(args.catalog, args.model or DEFAULT_MODELS, args.output)
    result = pack_corpus(args.dataset, args.split, args.output)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()

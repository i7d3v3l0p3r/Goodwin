"""Pack a held-out 16 kHz PCM corpus and matching catalog models for Goodwin's wake benchmark."""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import shutil
import wave

RATE = 16_000
FRAME = 1280
DEFAULT_MODELS = ("goodwin-standard", "goodwin-micro", "goodwin-micro-int8")


def pack_corpus(dataset, split, output):
    counts = Counter()
    duration_samples = 0
    with (output / "clips.jsonl").open("w", encoding="utf-8", newline="\n") as manifest, (
        output / "clips.pcm"
    ).open("wb") as pcm, (dataset / "manifest.jsonl").open(encoding="utf-8") as source:
        for line in source:
            clip = json.loads(line)
            if clip["split"] != split:
                continue
            with wave.open(str(dataset / clip["path"]), "rb") as audio:
                if (audio.getframerate(), audio.getnchannels(), audio.getsampwidth()) != (RATE, 1, 2):
                    raise ValueError(f"Expected 16 kHz mono PCM16: {clip['path']}")
                samples = audio.getnframes()
                if samples == 0:
                    raise ValueError(f"Empty clip: {clip['path']}")
                data = audio.readframes(samples)
                if len(data) != samples * 2:
                    raise ValueError(f"Incomplete clip: {clip['path']}")
            window = None
            minimum_samples = samples
            category = clip["category"]
            if category == "positive":
                onset, offset = clip["phrase_onset_s"], clip["phrase_offset_s"]
                if not 0 <= onset <= offset:
                    raise ValueError(f"Invalid phrase window: {clip['path']}")
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
    if not counts or not counts["positive"]:
        raise ValueError(f"No labelled evaluation set for split {split}")
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
    parser.add_argument("--split", default="test")
    parser.add_argument("--model", action="append", help="Catalog model ID, repeat for every runtime")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    copy_models(args.catalog, args.model or DEFAULT_MODELS, args.output)
    result = pack_corpus(args.dataset, args.split, args.output)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()

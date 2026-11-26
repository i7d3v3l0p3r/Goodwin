"""Validate and select the same local wake corpus for export, training and evaluation."""

from collections import defaultdict
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
import warnings
import wave

RATE = 16_000
SPLITS = ("train", "validation", "test")


@dataclass(frozen=True)
class Recording:
    meta: dict
    pcm_sha256: str


@dataclass(frozen=True)
class Corpus:
    root: Path
    recordings: list[Recording]
    splits: dict[str, list[Recording]]
    duplicates: list[dict]


def read_pcm(root: Path, row: dict) -> bytes:
    path = root / row["path"]
    with wave.open(str(path), "rb") as audio:
        frames = audio.getnframes()
        if (audio.getframerate(), audio.getnchannels(), audio.getsampwidth()) != (RATE, 1, 2) or not frames:
            raise ValueError(f"Expected nonempty 16 kHz mono PCM16: {path}")
        pcm = audio.readframes(frames)
    duration = frames / RATE
    declared = row["duration_s"]
    if len(pcm) != frames * 2 or not math.isfinite(declared) or abs(duration - declared) > 0.002:
        raise ValueError(f"Invalid WAV duration: {path}")
    if row["label"] == 1 and not 0 <= row["phrase_onset_s"] < row["phrase_offset_s"] <= duration + 0.05:
        raise ValueError(f"Invalid wake word boundaries: {path}")
    return pcm


def verified_pcm(corpus: Corpus, recording: Recording) -> bytes:
    pcm = read_pcm(corpus.root, recording.meta)
    if hashlib.sha256(pcm).hexdigest() != recording.pcm_sha256:
        raise ValueError(f"WAV changed after corpus validation: {recording.meta['path']}")
    return pcm


def load_corpus(root: Path) -> Corpus:
    recordings = []
    paths = set()
    groups = {}
    by_audio = defaultdict(list)
    with (root / "manifest.jsonl").open(encoding="utf-8") as manifest:
        for line in manifest:
            row = json.loads(line)
            path, group, split = row["path"], row["group"], row["split"]
            relative = Path(path)
            if not path or relative.anchor or ".." in relative.parts:
                raise ValueError(f"Expected a path relative to the corpus: {path}")
            if (split not in SPLITS or row["label"] not in (0, 1) or not group
                    or (row["category"] == "positive") != (row["label"] == 1)):
                raise ValueError(f"Invalid split, label, category or group: {path}")
            if path in paths:
                raise ValueError(f"Repeated manifest path: {path}")
            paths.add(path)
            if groups.setdefault(group, split) != split:
                raise ValueError(f"Voice/source group spans multiple splits: {group}")
            recording = Recording(row, hashlib.sha256(read_pcm(root, row)).hexdigest())
            recordings.append(recording)
            by_audio[recording.pcm_sha256].append(recording)

    splits = {split: [] for split in SPLITS}
    duplicates = []
    for copies in by_audio.values():
        if len({clip.meta["label"] for clip in copies}) != 1:
            duplicates.extend({"path": clip.meta["path"], "reason": "conflicting_label"} for clip in copies)
            continue
        # Keep earlier splits first so known training audio can never become held-out audio.
        # Use a stable representative within each split regardless of manifest order.
        first, *rest = sorted(copies, key=lambda clip: (SPLITS.index(clip.meta["split"]), clip.meta["path"]))
        splits[first.meta["split"]].append(first)
        duplicates.extend({"path": clip.meta["path"], "duplicate_of": first.meta["path"]} for clip in rest)
    for items in splits.values():
        items.sort(key=lambda clip: clip.meta["path"])
    return Corpus(root, recordings, splits, sorted(duplicates, key=lambda item: item["path"]))


def check_heldout(training: Corpus, heldout: Corpus) -> None:
    used = [clip for clip in training.recordings if clip.meta["split"] != "test"]
    test = heldout.splits["test"]
    groups = {clip.meta["group"] for clip in used} & {clip.meta["group"] for clip in test}
    audio = {clip.pcm_sha256 for clip in used} & {clip.pcm_sha256 for clip in test}
    if groups or audio:
        raise ValueError(f"Held-out corpus overlaps training or validation: {len(groups)} groups, {len(audio)} PCM hashes")


def training_fingerprint(corpus: Corpus) -> str:
    digest = hashlib.sha256()
    for split in ("train", "validation"):
        for clip in corpus.splits[split]:
            meta = clip.meta
            record = [split, clip.pcm_sha256, *(meta.get(key) for key in (
                "group", "category", "label", "phrase_onset_s", "phrase_offset_s",
            ))]
            digest.update(json.dumps(record, ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n")
    return digest.hexdigest()


def check_fingerprint(expected: str | None, current: str, artifact: str) -> bool:
    if expected is None:
        warnings.warn(f"{artifact} predates corpus fingerprints; its training data cannot be verified", stacklevel=2)
        return False
    if expected != current:
        raise ValueError(f"{artifact} uses different training or validation data")
    return True

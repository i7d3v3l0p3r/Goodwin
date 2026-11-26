"""Export Goodwin Micro to INT8 TFLite and verify it on the held-out audio."""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

import numpy as np
from ai_edge_litert.interpreter import Interpreter
from livekit.wakeword_train import data, micro
from livekit.wakeword_train.export import calibrate

from wakeword_eval_livekit import (
    export_checkpoint, score_streaming, select_threshold, summarize, write_test,
)
from wakeword_manifest import Corpus, check_fingerprint, check_heldout, load_corpus, training_fingerprint
from wakeword_train_livekit import clips_for, local_training_data


class CalibrationSpeech:
    def __init__(self, clips: list[np.ndarray]) -> None:
        self.clips = clips

    def crop(self, rng: np.random.Generator, length: int) -> np.ndarray:
        # Sample local speech into a representative stream, including room noise between words.
        parts = []
        samples = 0
        while samples < length:
            clip = self.clips[rng.integers(len(self.clips))]
            parts.append(clip)
            samples += len(clip)
        return np.concatenate(parts)[:length]

    def stop(self, join: bool = False) -> None:
        pass


def quantize(args: argparse.Namespace, calibration: dict, corpus: Corpus, fingerprint: str) -> Path:
    det, _, _, _ = export_checkpoint(args, fingerprint)
    calibrate(det, calibration["calibration_logit"], temperature=calibration["output_temperature"])
    model = micro.StreamingMicro(micro._export_form(det)).eval()
    train = clips_for(corpus, "train")
    sets, noise, speech = local_training_data(train)
    ambient = CalibrationSpeech(speech[0] + speech[1])
    minimum_noise_samples = 16 * 320000
    if len(noise) < minimum_noise_samples:
        noise = np.resize(noise, minimum_noise_samples)
    with TemporaryDirectory(prefix="goodwin-micro-") as temp:
        # The converter rewrites its input graph in place.
        onnx = Path(temp) / "goodwin_micro.onnx"
        shutil.copyfile(args.out / onnx.name, onnx)
        with (
            patch.object(data, "load_clipsets", lambda _: sets),
            patch.object(data, "noise_pool", lambda: noise),
            patch.object(data, "negative_speech_files", lambda: []),
            patch.object(data, "StreamPool", lambda *a, **kw: ambient),
        ):
            return micro.convert_micro_tflite(
                model, onnx, args.out / "goodwin_micro.tflite", str(args.dataset),
            )


def evaluate(args: argparse.Namespace, model: Path, budget: int, corpus: Corpus) -> None:
    fingerprint = training_fingerprint(corpus)
    interpreter = Interpreter(model_path=str(model))
    runner = interpreter.get_signature_runner()
    inputs, outputs = runner.get_input_details(), runner.get_output_details()
    if any(t["dtype"] != np.int8 for t in [*inputs.values(), *outputs.values()]):
        raise ValueError("Expected fully quantized INT8 inputs and outputs")
    if any(not np.issubdtype(t["dtype"], np.integer) for t in interpreter.get_tensor_details()):
        raise ValueError("Unexpected non-integer tensor in the quantized model")
    if micro.read_mel_params(model) is None:
        raise ValueError("Missing LiveKit log-mel metadata")
    validation = score_streaming(
        model, clips_for(corpus, "validation"), args.out / "tflite_validation_scores.jsonl",
        "micro",
    )
    # Score thresholds for TFLite lie on the int8 output grid.
    scale, zero = outputs["output_0"]["quantization"]
    grid = sorted({(q - zero) * scale for q in range(-128, 128) if 0 < (q - zero) * scale < 1})
    lower = select_threshold(validation, budget)["threshold"]
    threshold = next((point for point in grid if point >= lower), 1.0)
    val_result = summarize(validation, threshold)
    if val_result["false_alarm_clips"] > budget:
        raise ValueError("TFLite model exceeded the validation false-alarm budget")
    (args.out / "tflite_calibration.json").write_text(json.dumps({
        "score_threshold": threshold, "validation": val_result, "corpus_sha256": fingerprint,
    }, indent=2) + "\n", encoding="utf-8")
    check_heldout(corpus, corpus)
    test = score_streaming(model, clips_for(corpus, "test"), args.out / "tflite_test_scores.jsonl", "micro")
    result = write_test(test, threshold, args.out / "tflite_test_predictions.csv")
    (args.out / "tflite_evaluation.json").write_text(json.dumps({
        "score_threshold": threshold, "validation": val_result, "test": result,
        "corpus_sha256": fingerprint,
        "model_size_bytes": model.stat().st_size, "input_count": len(inputs),
        "output_count": len(outputs),
    }, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", type=Path, default=Path("datasets/goodwin"))
    parser.add_argument("--encoders", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.set_defaults(tier="micro")
    args = parser.parse_args()
    calibration = json.loads((args.out / "calibration.json").read_text(encoding="utf-8"))
    corpus = load_corpus(args.dataset)
    fingerprint = training_fingerprint(corpus)
    check_fingerprint(calibration.get("corpus_sha256"), fingerprint, "ONNX calibration")
    model = quantize(args, calibration, corpus, fingerprint)
    evaluate(args, model, calibration["validation_false_clip_budget"], corpus)

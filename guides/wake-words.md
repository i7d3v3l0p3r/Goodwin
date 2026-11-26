# Wake-word training and evaluation

To install a published model, use the [Mini 3 settings](mini-3.md#voice-and-microphone-settings) or the [ESP32-S3 guide](../firmware/esp32s3/README.md). To train your own, start from the repository root.

## Train the Russian wake word

Goodwin supports streaming **LiveKit WakeWord v2 Standard and Micro** ONNX models. Train and evaluate the Russian phrase using [LiveKit's trainer](https://github.com/livekit/livekit-wakeword/blob/v2.1.3/client/python/TRAINING.md) on a CUDA computer or an Apple silicon Mac. The stock trainer generates speech through LiveKit Inference and requires LiveKit Cloud credentials for that step.

```sh
uv tool install --python 3.13 "livekit-wakeword[train]==2.1.3"
livekit-wakeword setup --wakeword "Гудвин" --language ru --eval \
  --hard-negative "Гарвин" --hard-negative "Гудман" --hard-negative "будни"
livekit-wakeword train --wakeword "Гудвин" --type standard --out ./models
livekit-wakeword eval --wakeword "Гудвин"
```

Copy the exported ONNX file to `goodwin-data/goodwin.onnx`. Check activation with real recordings from the Station, including a question spoken immediately after the name, a name without a question, music, background conversation, and requests to Alice.

### Fine-tune on a local dataset

The scripts below use the WAV files and split labels in `datasets/goodwin/manifest.jsonl`. They use LiveKit's pretrained Standard encoder, streaming trainer, and ONNX exporter. All augmentation audio comes from the training split, and only the validation split is used for checkpoint and threshold selection.

Export, training, calibration and benchmark preparation use the same validated
manifest. Voice or source groups belong to one split. Exact PCM duplicates keep
one copy in the earliest split in `train`, `validation`, `test` order. Copies
with conflicting labels are excluded. Evaluation rejects an external test split
whose source groups or PCM hashes overlap training or validation. New checkpoints
record a fingerprint of the training and validation data, and evaluation checks
it before calibration. Older checkpoints can be evaluated with a provenance warning.

Extract the [v2.1.3 source archive](https://github.com/livekit/livekit-wakeword/archive/refs/tags/v2.1.3.zip) under `target/wakeword-v2/`. Download the [official encoder archive](https://github.com/livekit/livekit-wakeword/releases/download/v2.0.0/encoders.tar.gz), check the SHA-256 against `ENCODERS_RELEASE` in LiveKit's `encoders.py`, then extract its `encoders/` at the same location. Run these commands from the repository root in a Python 3.12 virtual environment.

```powershell
git apply --directory=target/wakeword-v2/livekit-wakeword-2.1.3 scripts/livekit-wakeword-cpu.patch
python -m pip install --index-url https://download.pytorch.org/whl/cpu torch==2.10.0+cpu torchaudio==2.10.0+cpu
python -m pip install --no-deps ./target/wakeword-v2/livekit-wakeword-2.1.3
python -m pip install numpy pydantic pyyaml librosa safetensors onnx onnxruntime matplotlib
$out = "datasets/goodwin/training/livekit-standard-v2"
python scripts/wakeword_train_livekit.py --encoders target/wakeword-v2/encoders --out $out
python scripts/wakeword_plot_livekit.py --run $out --training-only
python scripts/wakeword_eval_livekit.py --encoders target/wakeword-v2/encoders --out $out
python scripts/wakeword_plot_livekit.py --run $out
```

The export is `goodwin.onnx`. The evaluator streams audio through the LiveKit runtime and writes `evaluation.json`, per-clip predictions, and score traces. The threshold budget is 1% of negative validation clips, and the exporter maps the selected score to 0.50. The evaluation reports false alarms per clip and per hour of negative test audio. It counts the first activation in the annotated wake-word window as a hit.

For another wake word, pass its corpus with `--dataset` to both scripts, its text with `--phrase` to the trainer, and the export file stem with `--name` to the evaluator. Use a separate `--out` directory for each run.

Use the selected Standard checkpoint as the teacher for the smaller Micro model. The Micro ONNX export uses `onnxsim` to fold padding into constants for the Goodwin runtime.

```powershell
python -m pip install onnxsim==0.8.1
$micro = "datasets/goodwin/training/livekit-micro-v2"
python scripts/wakeword_train_livekit.py --tier micro --teacher "$out/selected.pt" --encoders target/wakeword-v2/encoders --out $micro
python scripts/wakeword_eval_livekit.py --tier micro --encoders target/wakeword-v2/encoders --out $micro
python scripts/wakeword_plot_livekit.py --run $micro
```

The Micro ONNX takes log-mel frames and recurrent state. Use LiveKit's `MicroRuntime` for Python evaluation. Goodwin computes the Micro frontend from PCM in Rust and reads the normalization parameters embedded in the ONNX.

For the INT8 export, install LiveKit's conversion dependencies. The converter uses audio from the local training split to calibrate quantization ranges. The script selects the INT8 trigger threshold on validation and reports its test results separately.

```powershell
python -m pip install tensorflow==2.21.0 tf-keras==2.21.0 onnx2tf==1.28.8 onnxsim==0.8.1 ai-edge-litert==2.3.0 onnx-graphsurgeon==0.6.1 sng4onnx==2.0.1 psutil
python scripts/wakeword_quantize_livekit.py --encoders target/wakeword-v2/encoders --out $micro
```

ESP32 firmware can run the wake word locally with the LiveKit ESP-IDF component. Embed `goodwin-micro-int8.tflite` in flash and use its catalog `score_threshold`. Call `lkww_process()` from an audio processing task with 16 kHz mono PCM to avoid the separate task and one-second audio buffer used by `lkww_feed()`. Check the arena allocation and `lkww_last_step_us()` with Wi-Fi and audio active on the target board. Each 1,280-sample inference step adds 80 ms of audio. [LiveKit ESP-IDF component](https://github.com/livekit/livekit-wakeword/tree/v2.1.3/client/esp32) · [Goodwin-wakeword releases](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest)

## Benchmark wake runtimes

Build the Linux binary with `wake-bench` in addition to the desired wake
runtimes. Prepare the held-out inputs from an existing dataset and release catalog.

```sh
python scripts/prepare-wake-benchmark.py datasets/goodwin /path/to/release/metadata.json target/wake-benchmark
goodwin wake-bench target/wake-benchmark/goodwin-standard.onnx target/wake-benchmark/goodwin-standard.json quiet
goodwin wake-bench target/wake-benchmark/goodwin-standard.onnx target/wake-benchmark/goodwin-standard.json target/wake-benchmark
```

Use `quiet` for three fresh processes per model to compare load time, warm RSS,
peak RSS and inference time per 80 ms frame. The corpus run resets the runtime
between clips and reports hits and false alarms at both the default Goodwin
sensitivity and the catalog threshold. Compare the same corpus digest and
report whether audio came from files or the physical microphone. Use a separate
build with `hotpath-alloc` for Rust allocation measurements.

The synthetic compatibility check uses fixed PCM frames and the published
Goodwin Standard ONNX, Micro ONNX and Micro INT8 models. It verifies the model
hashes, frame scores, thresholds and reset behavior. It measures runtime
compatibility across builds. Use a held-out dataset to measure recognition
accuracy.

```sh
python scripts/build-wake-tflite.py
python scripts/prepare-wake-reference.py
GOODWIN_WAKE_REFERENCE_DIR="$PWD/target/wake-reference" \
  cargo test --locked --features wake-tflite -- --ignored --exact wake::tests::published_models_match_the_synthetic_reference
```

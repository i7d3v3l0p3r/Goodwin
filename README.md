# Goodwin

Goodwin is an alternative voice assistant for Yandex Station Mini 3 (`bergamot`). It runs separately from Alice on a prepared Station. Say “Гудвин” followed by a question in one breath, or send a message from the web panel.

The Station detects the wake word locally, sends the recording to OpenRouter for transcription, generation and speech synthesis, and plays the answer. Each activation handles one voice request. Numbered timer choices carry over to the next voice request. The current web conversation lives in memory and clears when you start a new chat or stop Goodwin.

Download the desktop installer, Station package and plugins from [GitHub Releases](https://github.com/i7d3v3l0p3r/Goodwin/releases). See the [changelog](CHANGELOG.md) for release notes.

## Desktop installer

The Rust installer runs on Windows, Linux and macOS. It installs Goodwin on a prepared Yandex Station Mini 3 over network ADB with root access and support for the `/data/init.d` service directory. Install [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) and put ADB on your `PATH`, or select the ADB executable under **Дополнительные настройки**.

1. Select a Station from the list. The installer starts an mDNS search for Mini 3 on launch. Click **Обновить** to search again or **Указать адрес вручную** to enter an IP address or hostname. Add an ADB port if it differs from 5555.
2. Click **Установить или обновить**. The installer downloads the latest stable Station package from GitHub, verifies its SHA-256, starts the service and enables it at boot.
3. Open the panel from the installer and create a password on a fresh installation.

For local testing, open **Дополнительные настройки → Локальная сборка** and select an archive or a directory containing `goodwin-mini3-aarch64.tar.gz` or an extracted package. An unpackaged test build needs `goodwin`, both ONNX files and the model attribution files in `models/`. If the directory contains `SHA256SUMS`, make sure it matches the local build. Click **Сбросить** in **Локальная сборка** to resume installing from GitHub.

Updates to the managed Station service reuse its data directory at `/data/opt/etc/goodwin` and keep its wake model selection by default. If the new service fails the startup check, the installer restores the previous executable and models and restarts the previous service if it was running.

To build and launch the desktop installer locally, run:

```sh
cargo run --locked --manifest-path installer/Cargo.toml --release
```

The installer is a separate Rust project and builds without the Goodwin web panel or ARM cross-compiler. On Linux it uses OpenGL with Wayland or X11, and the native folder picker uses the desktop portal.

## Build

Use Rust, Node.js 22.12+ or 24 LTS, and npm.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked --release
```

The panel and Russian offline error announcement are embedded in the binary. On Windows, use the Station build script with Zig and Python on your path.

```powershell
rustup target add aarch64-unknown-linux-musl
.\scripts\build-station.ps1
```

On Linux, install an AArch64 musl C toolchain or use [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) with Zig.

```sh
rustup target add aarch64-unknown-linux-musl
cargo zigbuild --locked --release --target aarch64-unknown-linux-musl
```

The Station binary is `target/aarch64-unknown-linux-musl/release/goodwin`. The native binary runs the panel on a development computer. The live microphone and playback tests require a Station or an audio test fixture.

With the trained Standard and Micro models in `datasets/goodwin/training/`, run
`python scripts/package-station.py` to package the ARM64 binary, both ONNX files,
and the launcher in `target/goodwin-mini3-aarch64.tar.gz`.
The packager takes `LICENSE` and `NOTICE` from the [extracted LiveKit encoders](#fine-tune-on-a-local-dataset) in
`target/wakeword-v2/encoders/`. Pass `--encoders <directory>` if you extracted them elsewhere.

To package the wake-word models shipped with Goodwin, use `python scripts/package-station.py --standard models/standard.onnx --micro models/micro.onnx --encoders models`.

## Publish a release

Set the Goodwin version in `Cargo.toml`, update `Cargo.lock`, and add an entry headed `## [<version>] - YYYY-MM-DD` to `CHANGELOG.md` with English and Russian notes. Push the commit, then push a `v<version>` tag on it. GitHub Actions builds the ARM64 Station package, both plugin archives and the desktop installers, runs the tests, and publishes the downloads and `SHA256SUMS` with the matching changelog entry.

To start the workflow manually for an existing release tag, run `gh workflow run release.yml --ref v<version>`.

## Run on the Station

Copy the Linux binary to a writable, executable directory on a prepared Mini 3. Leave the Station's existing voice services running. Goodwin connects to `yio_audio` at `127.0.0.1:9850` and uses `aplay -D speakers`. Its configuration and model go in a separate data directory.

```sh
chmod +x ./goodwin
./goodwin setup ./goodwin-data
./goodwin serve ./goodwin-data 0.0.0.0:8787
```

Open the Station's address on port 8787 and create a panel password. Goodwin signs you in so you can add an OpenRouter API key in **Settings**. Keep the panel on a trusted network. It uses plain HTTP, and the first visitor who completes setup sets the password. For explicit password-free access, run `goodwin setup ./goodwin-data open` when initializing a new data directory.

To enable voice activation, place a LiveKit v2 Standard or Micro ONNX at `goodwin-data/goodwin.onnx` and restart the service. Goodwin selects the input format from the ONNX graph. Run `goodwin wake-check model.onnx` to check loading, warm-up, and one inference step without the microphone.

On the Station, extract the package with `gzip -dc goodwin-mini3-aarch64.tar.gz | tar -xf -`. In the extracted directory, run `./start.sh standard` or `./start.sh micro` to select the model at startup. Both use the same data directory and panel settings. Use `GOODWIN_PANEL_ADDRESS=0.0.0.0:8787` to open the panel to your trusted network and `GOODWIN_DATA_DIR` to reuse an existing data directory.

The text chat and push-button Station test work without a wake model. In Settings you can pause background detection, change sensitivity, test the speaker and microphone, and see timings for the last request. Pausing detection leaves the button test available. Goodwin keeps transcripts in the current conversation and does not save recordings or diagnostic transcripts to disk.

Use **Settings → System prompt** to define Goodwin’s role, personality, and tone for both text and voice replies. Save the prompt to apply it to subsequent requests. Clear the field and save to restore the built-in instructions. Goodwin keeps the instructions for spoken output and voice plugin calls when a custom prompt is set.

On a development computer, use the default loopback address.

```sh
goodwin setup goodwin-data open
goodwin serve goodwin-data
```

The panel opens at `http://127.0.0.1:8787`. When running Vite in development mode, keep Goodwin on that address for the API proxy.

| Environment variable | Purpose | Default |
| --- | --- | --- |
| `OPENROUTER_API_KEY` | API key when the panel has no saved key | unset |
| `GOODWIN_MICROPHONE` | Local `yio_audio` socket address | `127.0.0.1:9850` |
| `GOODWIN_MIC_GAIN` | PCM input gain before detection and capture | `4` |
| `GOODWIN_WAKE_MODEL` | Path to a Standard or Micro ONNX wake model | `<data-dir>/goodwin.onnx` |
| `GOODWIN_APLAY` | ALSA playback program path | `aplay` |
| `GOODWIN_PLAYBACK_DEVICE` | ALSA playback device | `speakers` |

## Plugins

Sign in to the panel and use **Plugins → Upload package** to install a trusted `.tar.gz` archive or update an existing plugin. When the process reports readiness, its page, navigation and API become available. A failed update restores the previous version and its data. Removing a plugin also removes its saved data.

If you previously set up open access, stop Goodwin, run `goodwin set-password <data-dir>`, then start it again. Your saved settings and API key are retained.

The Windows Station build produces `target/plugins/probe-linux-aarch64.tar.gz` and `target/plugins/timers-linux-aarch64.tar.gz`. Use the probe to check installation and the counter on its page. Upload Timers to create, list and cancel timers in the panel. For voice commands, select an OpenRouter answer model that supports tool calling. Panel timers and their sounds run locally.

See the [plugin development guide](plugins/README.md) for the package format, process protocol, and UI kit.

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

## Check the cloud pipeline with a recording

Set `OPENROUTER_API_KEY` and supply a Russian WAV recording.

```sh
goodwin wav question.wav answer.wav
```

This runs transcription, generation and speech synthesis with the default models and saves the answer as WAV. For this command the environment variables `GOODWIN_STT_MODEL`, `GOODWIN_CHAT_MODEL`, `GOODWIN_TTS_MODEL` and `GOODWIN_TTS_VOICE` override the defaults. In the panel, use Settings to select models and the voice. Choose a speech model that supports raw 16-bit PCM.

## Local verification

```sh
npm run build --prefix web
cargo test --locked --workspace
```

The tests emulate a Station microphone and the OpenRouter transport on loopback and capture the generated audio without playing it.

The panel embeds [Inter](https://rsms.me/inter/) under the [SIL Open Font License](web/Inter-OFL.txt).
